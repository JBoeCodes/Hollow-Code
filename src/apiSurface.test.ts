import { describe, expect, it } from 'vitest';
// Read as text (like navigation.test.ts reads App.tsx), so this stays a pure
// node test that never loads Tauri.
import API_TS from './api.ts?raw';
import MOCK_API_TS from './mockApi.ts?raw';

/**
 * Dead frontend wrappers (merged plan slice 6, decision 7). Activity's
 * cross-conversation question list moved to `list_needs_you` in wave 2, which
 * left `api.listOpenInteractions` with no caller. The Tauri command stays: it
 * is an agent app tool (`app_actions.rs`, area "interactions").
 */
describe('api surface', () => {
  it('has no wrapper or mock for list_open_interactions', () => {
    expect(API_TS).not.toMatch(/listOpenInteractions/);
    expect(API_TS).not.toMatch(/['"]list_open_interactions['"]/);
    expect(MOCK_API_TS).not.toMatch(/['"]list_open_interactions['"]/);
  });

  it('keeps the per-conversation question-card wrapper the composer uses', () => {
    expect(API_TS).toMatch(/listPendingInteractions\s*:/);
  });
});
