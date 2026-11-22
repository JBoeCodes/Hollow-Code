import { expect, it } from 'vitest';
import SETTINGS from './pages/Settings.tsx?raw';

// P0-24: Settings, Connectors' Hollow app switch says what agents can do with
// it (they change Hollow, not only read it) and when they ask first, and makes
// no claim of a call log that no view shows.
const row = SETTINGS.slice(SETTINGS.indexOf('<strong>Hollow app</strong>'), SETTINGS.indexOf('label="Hollow app tools"'));
const copy = row.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim();

it('names changing Hollow and the approval rule', () => {
  expect(row.length).toBeGreaterThan(0);
  expect(copy).toContain('Agents can see and change Hollow');
  expect(copy).toContain('builds and installs');
  expect(copy).toContain('ask you first');
  expect(copy).toContain('Bypass');
});

it('makes no unlinked "logged" claim', () => {
  expect(copy).not.toMatch(/\blogged\b/i);
  expect(copy).not.toMatch(/read what you see/i);
});
