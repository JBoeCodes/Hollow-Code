import { describe, expect, it } from 'vitest';
import { emptyScene, emptyStateScene } from './emptyScene';
it('shared EmptyState suppresses art for incomplete or filtered coverage', () => {
  for (const kind of ['tasks', 'automations'] as const) {
    expect(emptyStateScene(kind, {ready:true})).toBe(kind);
    for (const evidence of [{ready:false},{ready:true,failed:true},{ready:true,partial:true},{ready:true,filtered:true}])
      expect(emptyStateScene(kind, evidence)).toBe(null);
  }
});
describe('empty view evidence before decoration', () => {
  it('keeps every complete empty view distinct', () => {
    for (const kind of ['inbox','commitments','results','sources','mission','search','waiting'] as const)
      expect(emptyScene(kind, { ready: true })).toBe(kind);
  });
  it('does not turn loading, failed, partial or unknown coverage into all clear', () => {
    expect(emptyScene('waiting', {ready:false})).toBe(null);
    expect(emptyScene('inbox', {ready:true, failed:true})).toBe(null);
    expect(emptyScene('sources', {ready:true, partial:true})).toBe(null);
  });
  it('uses the neutral search scene for filtered results, but never for a failed read', () => {
    expect(emptyScene('commitments', {ready:true, filtered:true})).toBe('search');
    expect(emptyScene('results', {ready:true, filtered:true, failed:true})).toBe(null);
  });
});

import { recordingsMissing } from './emptyScene';
it('an unconfigured recording connector is not a failed source; recording failures affect only their filters', () => {
  expect(recordingsMissing('notConnected','all')).toBe(false);
  expect(recordingsMissing('unavailable','task')).toBe(false);
  expect(recordingsMissing('unavailable','all')).toBe(true);
  expect(recordingsMissing('unavailable','recording')).toBe(true);
});
