import { describe, expect, it } from 'vitest';
import { inspectionForTab } from './previewCursor';

describe('Preview inspection identity', () => {
  const event = { tabId: 'a', gen: 3, seq: 1, visible: true };
  it('matches only the same settled native navigation', () => {
    expect(inspectionForTab(event, { id: 'a', gen: 3, loading: false })).toBe(true);
    expect(inspectionForTab(event, { id: 'b', gen: 3, loading: false })).toBe(false);
    expect(inspectionForTab(event, { id: 'a', gen: 4, loading: false })).toBe(false);
    expect(inspectionForTab(event, { id: 'a', gen: 3, loading: true })).toBe(false);
    expect(inspectionForTab(event, undefined)).toBe(false);
  });
});
