import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DECK_PATH, LEASE_PATH, mockConnectedFiles, mockDirectoryState, mockDirectoryStatus, mockReadConnectedFile } from './mockDirectories';

const NOW = '2026-07-15T13:00:00-04:00';

describe('browser-mock connected inventory (next-level P07)', () => {
  const storage = new Map<string, string>();
  beforeEach(() => {
    storage.clear();
    vi.stubGlobal('localStorage', { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => storage.set(k, v) });
    vi.stubGlobal('location', { search: '?mock=1' });
  });
  it('the relevant fixture after item 2,500 is listed and findable by name', () => {
    const state = mockDirectoryState(NOW);
    const index = state.files.findIndex((f: { path: string }) => f.path === LEASE_PATH);
    expect(index).toBeGreaterThan(2500);
    const hit = mockConnectedFiles(state, { query: 'garage 14 lease', limit: 8 });
    expect(hit.total).toBe(1);
    expect(hit.files[0].path).toBe(LEASE_PATH);
    expect(mockReadConnectedFile(state, LEASE_PATH)).toContain('escalator');
  });
  it('pages instead of returning everything, and status never carries files', () => {
    const state = mockDirectoryState(NOW);
    const first = mockConnectedFiles(state, { limit: 100 });
    expect(first.files).toHaveLength(100);
    expect(first.total).toBe(2804);
    expect(mockConnectedFiles(state, { offset: 2800, limit: 100 }).files).toHaveLength(4);
    const status = mockDirectoryStatus(state);
    expect(status.files).toBeUndefined();
    expect(status.coverage).toMatchObject({ indexed: 2804, available: 2804, omitted: 0, tooLarge: 1, needsReader: 0, extracted: 1 });
    // R14a: the mock PDF is read by Hollow and opens to its text.
    expect(mockConnectedFiles(state, { fileState: 'extracted' }).files.map(f => f.path)).toEqual([DECK_PATH]);
    expect(mockReadConnectedFile(state, DECK_PATH)).toMatch(/^## Page 1\nQ3 regional review\n/);
    expect(mockConnectedFiles(state, { fileState: 'tooLarge' }).files.map(f => f.relative)).toEqual(['Launch/exports/ledger-2025.csv']);
    expect(() => mockReadConnectedFile(state, '/Hollow/Projects/Launch/exports/ledger-2025.csv')).toThrow(/larger than the 1 MB/);
  });
  it('a seeded legacy fixture keeps working (readable flag only)', () => {
    localStorage.setItem('mockDirectories', JSON.stringify({ version: 1, projects: [], warnings: [], refreshedAt: NOW, files: [{ path: '/p/A/deck.key', relative: 'A/deck.key', projectId: 'a', size: 1, readable: false }] }));
    const page = mockConnectedFiles(mockDirectoryState(NOW));
    expect(page.files[0].state).toBe('needsReader');
  });
});
