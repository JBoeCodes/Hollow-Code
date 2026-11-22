import { describe, expect, it } from 'vitest';
import { filterOptions, groupOptions, nextEnabled, typeaheadMatch, type SelectOption } from './selectModel';

const opts: SelectOption[] = [
  { value: 'brief', label: 'Brief', group: 'Workspace' },
  { value: 'plan', label: 'Plan', group: 'Workspace' },
  { value: 'sources', label: 'Sources', group: 'Workspace', disabled: true },
  { value: 'outputs', label: 'Outputs', group: 'Workspace' },
  { value: 'files', label: 'Conversation files', group: 'This conversation', keywords: 'artifacts' },
  { value: 'visual', label: 'Visuals', group: 'This conversation', detail: 'Whiteboards and previews' },
];

describe('filterOptions', () => {
  it('keeps everything for an empty query and matches all terms anywhere', () => {
    expect(filterOptions(opts, '  ')).toHaveLength(6);
    expect(filterOptions(opts, 'conv fil').map(o => o.value)).toEqual(['files']);
    expect(filterOptions(opts, 'artifacts').map(o => o.value)).toEqual(['files']);
    expect(filterOptions(opts, 'whiteboards').map(o => o.value)).toEqual(['visual']);
    expect(filterOptions(opts, 'workspace').length).toBe(4);
  });
  it('ignores case and accents', () => {
    expect(filterOptions([{ value: 'a', label: 'Café notes' }], 'CAFE').length).toBe(1);
  });
});

describe('nextEnabled', () => {
  it('skips disabled options and clamps at the ends', () => {
    expect(nextEnabled(opts, 1, 1)).toBe(3);
    expect(nextEnabled(opts, 3, -1)).toBe(1);
    expect(nextEnabled(opts, 5, 1)).toBe(5);
    expect(nextEnabled(opts, -1, 1)).toBe(0);
    expect(nextEnabled(opts, opts.length, -1)).toBe(5);
    expect(nextEnabled([{ value: 'x', label: 'x', disabled: true }], -1, 1)).toBe(-1);
  });
});

describe('typeaheadMatch', () => {
  it('jumps to the next label with the prefix, wrapping, and cycles a repeated letter', () => {
    expect(typeaheadMatch(opts, 'p', -1)).toBe(1);
    expect(typeaheadMatch(opts, 'o', 1)).toBe(3);
    expect(typeaheadMatch(opts, 's', 0)).toBe(-1); // only match is disabled
    expect(typeaheadMatch(opts, 'vis', 0)).toBe(5);
    const many: SelectOption[] = [{ value: '1', label: 'Alpha' }, { value: '2', label: 'Beta' }, { value: '3', label: 'Apex' }];
    expect(typeaheadMatch(many, 'a', 0)).toBe(2);
    expect(typeaheadMatch(many, 'aa', 2)).toBe(0);
    expect(typeaheadMatch(many, 'ap', 0)).toBe(2);
  });
});

describe('groupOptions', () => {
  it('groups contiguous runs and keeps flat indices', () => {
    const g = groupOptions(opts);
    expect(g.map(x => x.group)).toEqual(['Workspace', 'This conversation']);
    expect(g[1].items.map(i => i.index)).toEqual([4, 5]);
  });
});
