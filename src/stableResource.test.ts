import { describe, expect, it } from 'vitest';
import type { Resource } from 'solid-js';
import { isFirstLoad, latestUnlessErrored, preserveDuringRefetch, savedOrRead, unlessErrored } from './stableResource';

describe('preserveDuringRefetch', () => {
  it('uses latest during refresh and forwards resource state', () => {
    let state: Resource<string>['state'] = 'ready';
    let current = 'ready value';
    let latest = 'ready value';
    let reads = 0;
    const raw = (() => {
      reads += 1;
      return current;
    }) as Resource<string>;
    Object.defineProperties(raw, {
      state: { get: () => state },
      loading: { get: () => state === 'pending' || state === 'refreshing' },
      error: { get: () => undefined },
      latest: { get: () => latest },
    });

    const stable = preserveDuringRefetch(raw);
    expect(stable()).toBe('ready value');
    expect(reads).toBe(1);

    state = 'refreshing';
    current = 'in-flight value';
    latest = 'last resolved value';
    expect(stable()).toBe('last resolved value');
    expect(stable.state).toBe('refreshing');
    expect(stable.loading).toBe(true);
    expect(reads).toBe(1);
  });

  it('never throws once errored: last good value, or undefined if there was none', () => {
    let state: Resource<string>['state'] = 'errored';
    let value: string | undefined;
    const raw = (() => {
      if (state === 'errored') throw new Error('Couldn’t read the tasks folder');
      return value;
    }) as Resource<string>;
    Object.defineProperties(raw, {
      state: { get: () => state },
      loading: { get: () => false },
      error: { get: () => state === 'errored' ? new Error('Couldn’t read the tasks folder') : undefined },
      latest: { get: () => { if (state === 'errored') throw new Error('Couldn’t read the tasks folder'); return value; } },
    });
    const stable = preserveDuringRefetch(raw);
    // A first read that failed: nothing to keep, and no throw.
    expect(stable()).toBeUndefined();
    expect(stable.error).toBeInstanceOf(Error);
    state = 'ready'; value = 'tasks v1';
    expect(stable()).toBe('tasks v1');
    // A later read that failed keeps the last good list on screen.
    state = 'errored';
    expect(stable()).toBe('tasks v1');
  });
});

describe('refreshFailed (X21)', () => {
  // Solid's own sequence for a refetch that fails, then a Retry that works:
  // ready → refreshing → errored (error set, reads throw) → refreshing (error
  // still set until the read settles) → ready (error cleared).
  function fake() {
    const s = { state: 'unresolved' as Resource<string[]>['state'], value: undefined as string[] | undefined, error: undefined as unknown };
    const raw = (() => { if (s.error && s.state === 'errored') throw s.error; return s.value; }) as Resource<string[]>;
    Object.defineProperties(raw, {
      state: { get: () => s.state },
      loading: { get: () => s.state === 'pending' || s.state === 'refreshing' },
      error: { get: () => s.error },
      latest: { get: () => { if (s.error && s.state === 'errored') throw s.error; return s.value; } },
    });
    return { s, stable: preserveDuringRefetch(raw) };
  }

  it('is set by a failed refetch while the last good value stays, and cleared by the next success', () => {
    const { s, stable } = fake();
    s.state = 'ready'; s.value = ['Morning Brief'];
    expect(stable()).toEqual(['Morning Brief']);
    expect(stable.refreshFailed).toBe(false);

    s.state = 'refreshing';
    expect(stable()).toEqual(['Morning Brief']);
    expect(stable.refreshFailed).toBe(false);

    s.state = 'errored'; s.error = 'mock list_workflows failure';
    expect(stable()).toEqual(['Morning Brief']);
    expect(stable.refreshFailed).toBe(true);

    // Retry in flight: still marked, still the old list.
    s.state = 'refreshing';
    expect(stable()).toEqual(['Morning Brief']);
    expect(stable.refreshFailed).toBe(true);

    s.state = 'ready'; s.error = undefined; s.value = ['Morning Brief', 'Weekly digest'];
    expect(stable()).toEqual(['Morning Brief', 'Weekly digest']);
    expect(stable.refreshFailed).toBe(false);
  });

  it('is not set when the first read failed: nothing earlier is on screen', () => {
    const { s, stable } = fake();
    s.state = 'errored'; s.error = new Error('no folder');
    expect(stable()).toBeUndefined();
    expect(stable.refreshFailed).toBe(false);
    expect(stable.error).toBeTruthy();
  });
});

describe('isFirstLoad', () => {
  const withState = (state: Resource<string>['state']) => {
    const raw = (() => 'v') as Resource<string>;
    Object.defineProperties(raw, {
      state: { get: () => state },
      loading: { get: () => state === 'pending' || state === 'refreshing' },
      error: { get: () => undefined },
      latest: { get: () => 'v' },
    });
    return raw;
  };

  it('is true only when there is no previous value to keep on screen', () => {
    expect(isFirstLoad(withState('unresolved'))).toBe(true);
    expect(isFirstLoad(withState('pending'))).toBe(true);
    expect(isFirstLoad(withState('ready'))).toBe(false);
    expect(isFirstLoad(withState('errored'))).toBe(false);
  });

  it('is false while refreshing, where `loading` would wrongly be true', () => {
    const refreshing = withState('refreshing');
    // This is the whole point: `loading` cannot be used to decide whether to
    // show a placeholder, because it blanks a row that still has good data.
    expect(refreshing.loading).toBe(true);
    expect(isFirstLoad(refreshing)).toBe(false);
  });
});

describe('unlessErrored (B·F08)', () => {
  // Like Solid: reading an errored resource throws.
  const resource = <T,>(value: T, error?: unknown) => {
    const read = (() => { if (error) throw error; return value; }) as (() => T) & { error: unknown; latest: T };
    Object.defineProperties(read, {
      error: { get: () => error },
      latest: { get: () => { if (error) throw error; return value; } },
    });
    return read;
  };
  it('passes the value through when the resource is fine', () => {
    expect(unlessErrored(resource(['a']))).toEqual(['a']);
    expect(latestUnlessErrored(resource({ state: 'ok' }))).toEqual({ state: 'ok' });
  });
  it('gives undefined instead of throwing once the read failed, so the page can show the error', () => {
    const failed = resource(['a'], 'Recording link store was damaged (…). It was moved to …');
    expect(() => failed()).toThrow();
    expect(() => failed.latest).toThrow();
    expect(unlessErrored(failed)).toBeUndefined();
    expect(latestUnlessErrored(failed)).toBeUndefined();
  });
});

describe('savedOrRead (H6 follow-up)', () => {
  it('answers refetch(saved) with the saved value at once, without reading', () => {
    let reads = 0;
    const fetcher = savedOrRead(async () => { reads += 1; return { chosen: 'from disk' }; });
    const saved = { chosen: 'claude-sonnet-5-5' };
    // Synchronous, not a promise: Solid makes it the newest load at once and
    // drops any read still in flight.
    expect(fetcher('claude#1', { value: { chosen: '' }, refetching: saved })).toBe(saved);
    expect(reads).toBe(0);
  });

  it('reads on a plain refetch and on a key change', async () => {
    let reads = 0;
    const fetcher = savedOrRead(async () => { reads += 1; return { chosen: 'from disk' }; });
    await expect(fetcher('claude#2', { value: undefined, refetching: true })).resolves.toEqual({ chosen: 'from disk' });
    await expect(fetcher('claude#3', { value: undefined, refetching: false })).resolves.toEqual({ chosen: 'from disk' });
    expect(reads).toBe(2);
  });
});
