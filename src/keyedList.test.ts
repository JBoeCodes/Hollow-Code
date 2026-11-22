import { describe, expect, it } from 'vitest';
import { keepUnchanged, keepUnchangedRecord, keyedList, samePayload } from './keyedList';

/** Stand-in for a wire row: scalars plus one nested array and one array of objects. */
type Row = {
  url: string;
  title: string;
  labels?: string[];
  subtasks?: { text: string; done: boolean }[];
};

/** A refetch hands back structurally identical objects with new references. */
const churn = <T,>(rows: T[]): T[] => JSON.parse(JSON.stringify(rows)) as T[];

const row = (url: string, title = url): Row => ({ url, title });

describe('samePayload', () => {
  it('is true for identical references without walking', () => {
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    // Would blow the stack if the reference fast path were missing.
    expect(samePayload(cyclic, cyclic)).toBe(true);
  });

  it('compares scalars by value', () => {
    expect(samePayload('a', 'a')).toBe(true);
    expect(samePayload(1, 1)).toBe(true);
    expect(samePayload(null, null)).toBe(true);
    expect(samePayload(null, undefined)).toBe(false);
    expect(samePayload(1, '1')).toBe(false);
  });

  it('compares nested arrays and objects structurally', () => {
    const a = { labels: ['x', 'y'], subtasks: [{ text: 't', done: false }] };
    expect(samePayload(a, churn([a])[0])).toBe(true);
    expect(samePayload(a, { labels: ['x'], subtasks: [{ text: 't', done: false }] })).toBe(false);
    expect(samePayload(a, { labels: ['x', 'y'], subtasks: [{ text: 't', done: true }] })).toBe(false);
  });

  it('does not confuse an array with an object', () => {
    expect(samePayload([], {})).toBe(false);
  });

  it('rejects an extra or renamed key rather than ignoring it', () => {
    expect(samePayload({ a: 1 }, { a: 1, b: 2 })).toBe(false);
    expect(samePayload({ a: 1, b: 2 }, { a: 1 })).toBe(false);
    expect(samePayload({ a: undefined }, { b: undefined })).toBe(false);
  });

  it('falls back to reference equality past the depth cap', () => {
    // 10 levels deep — beyond MAX_DEPTH (8), so equal-but-distinct reads as changed.
    const deep = (): unknown => ({ a: { a: { a: { a: { a: { a: { a: { a: { a: { a: 1 } } } } } } } } } });
    expect(samePayload(deep(), deep())).toBe(false);
    const shared = deep();
    expect(samePayload(shared, shared)).toBe(true);
  });
});

describe('keyedList', () => {
  it('keeps item identity when the payload is unchanged', () => {
    const first = [row('a'), row('b')];
    let src: Row[] = first;
    const keyed = keyedList(() => src, r => r.url);

    const before = keyed();
    src = churn(first);
    const after = keyed();

    expect(after).toHaveLength(2);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
    // The wrapper is what preserves identity; the source really did churn.
    expect(src[0]).not.toBe(before[0]);
  });

  it('preserves identity through nested arrays of objects', () => {
    const rich: Row[] = [{ url: 'a', title: 'A', labels: ['x'], subtasks: [{ text: 't', done: false }] }];
    let src = rich;
    const keyed = keyedList(() => src, r => r.url);

    const before = keyed();
    src = churn(rich);
    expect(keyed()[0]).toBe(before[0]);
  });

  it('hands out a new object when the payload genuinely changed', () => {
    let src: Row[] = [row('a', 'A'), row('b', 'B')];
    const keyed = keyedList(() => src, r => r.url);
    const before = keyed();

    src = [{ url: 'a', title: 'A renamed' }, row('b', 'B')];
    const after = keyed();

    expect(after[0]).not.toBe(before[0]);
    expect(after[0].title).toBe('A renamed');
    // A sibling that did not change must not be collateral damage.
    expect(after[1]).toBe(before[1]);
  });

  it('notices a nested change one level down', () => {
    let src: Row[] = [{ url: 'a', title: 'A', subtasks: [{ text: 't', done: false }] }];
    const keyed = keyedList(() => src, r => r.url);
    const before = keyed();

    src = [{ url: 'a', title: 'A', subtasks: [{ text: 't', done: true }] }];
    expect(keyed()[0]).not.toBe(before[0]);
  });

  it('keeps identity across a reorder', () => {
    const base = [row('a'), row('b'), row('c')];
    let src: Row[] = base;
    const keyed = keyedList(() => src, r => r.url);
    const before = keyed();

    src = churn([base[2], base[0], base[1]]);
    const after = keyed();

    expect(after.map(r => r.url)).toEqual(['c', 'a', 'b']);
    expect(after[0]).toBe(before[2]);
    expect(after[1]).toBe(before[0]);
    expect(after[2]).toBe(before[1]);
  });

  it('keeps existing identity across an insert', () => {
    const base = [row('a'), row('b')];
    let src: Row[] = base;
    const keyed = keyedList(() => src, r => r.url);
    const before = keyed();

    src = churn([base[0], row('new'), base[1]]);
    const after = keyed();

    expect(after.map(r => r.url)).toEqual(['a', 'new', 'b']);
    expect(after[0]).toBe(before[0]);
    expect(after[2]).toBe(before[1]);
  });

  it('keeps surviving identity across a removal and drops the evicted entry', () => {
    const base = [row('a'), row('b')];
    let src: Row[] = base;
    const keyed = keyedList(() => src, r => r.url);
    const before = keyed();

    src = churn([base[0]]);
    const afterRemoval = keyed();
    expect(afterRemoval).toHaveLength(1);
    expect(afterRemoval[0]).toBe(before[0]);

    // 'b' was evicted, so its return is a genuine arrival, not a cache hit.
    src = churn(base);
    const afterReturn = keyed();
    expect(afterReturn[0]).toBe(before[0]);
    expect(afterReturn[1]).not.toBe(before[1]);
  });

  it('never aliases one object across two rows sharing a key', () => {
    let src: Row[] = [row('dup', 'first'), row('dup', 'second')];
    const keyed = keyedList(() => src, r => r.url);
    const before = keyed();

    expect(before[0]).not.toBe(before[1]);
    expect(before[1].title).toBe('second');

    src = churn(src);
    const after = keyed();
    expect(after[0]).toBe(before[0]);
    expect(after[0]).not.toBe(after[1]);
    expect(after[1].title).toBe('second');
  });

  it('treats an undefined source as empty and recovers when it resolves', () => {
    let src: Row[] | undefined;
    const keyed = keyedList(() => src, r => r.url);

    expect(keyed()).toEqual([]);
    expect(keyed()).toEqual([]);

    src = [row('a')];
    const resolved = keyed();
    expect(resolved.map(r => r.url)).toEqual(['a']);

    src = undefined;
    expect(keyed()).toEqual([]);
  });

  it('handles an empty list without disturbing the next pass', () => {
    let src: Row[] = [];
    const keyed = keyedList(() => src, r => r.url);
    expect(keyed()).toEqual([]);

    src = [row('a')];
    const first = keyed();
    src = churn(first);
    expect(keyed()[0]).toBe(first[0]);
  });

  it('reuses the output array when the source array reference is unchanged', () => {
    let reads = 0;
    const base = [row('a')];
    const keyed = keyedList(() => { reads++; return base; }, r => r.url);

    const a = keyed();
    const b = keyed();
    expect(b).toBe(a);
    expect(reads).toBe(2); // the source is still read, so reactivity is preserved
  });
});

describe('keepUnchanged (P4)', () => {
  const a = { id: 'a', n: 1 }, b = { id: 'b', n: 2 };
  it('returns the earlier array when every item is equal and in place', () => {
    const prev = [a, b];
    expect(keepUnchanged(prev, [{ ...a }, { ...b }], x => x.id)).toBe(prev);
  });
  it('a reorder is a new array of the earlier objects', () => {
    const out = keepUnchanged([a, b], [{ ...b }, { ...a }], x => x.id);
    expect(out).toEqual([b, a]);
    expect(out[0]).toBe(b);
    expect(out[1]).toBe(a);
  });
  it('a changed, added or removed item makes a new array; duplicates never alias', () => {
    const out = keepUnchanged([a, b], [{ ...a }, { id: 'b', n: 3 }], x => x.id);
    expect(out[0]).toBe(a);
    expect(out[1]).toEqual({ id: 'b', n: 3 });
    expect(keepUnchanged([a, b], [{ ...a }], x => x.id)).toEqual([a]);
    const dup = keepUnchanged([a], [{ ...a }, { ...a }], x => x.id);
    expect(dup[0]).toBe(a);
    expect(dup[1]).not.toBe(a);
  });
  it('records: the earlier record when nothing changed; a gone key makes a new one', () => {
    const prev = { a, b };
    expect(keepUnchangedRecord(prev, { a: { ...a }, b: { ...b } })).toBe(prev);
    const fewer = keepUnchangedRecord(prev, { a: { ...a } });
    expect(fewer).toEqual({ a });
    expect(fewer.a).toBe(a);
    expect(keepUnchangedRecord({}, {})).toEqual({});
  });
});
