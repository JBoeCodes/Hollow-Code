import { describe, expect, it } from 'vitest';
import {
  AUTO_SCAN_AFTER_MS, completeness, groupSuggestions, headline, lastLooked, shouldAutoScan, sourceChip, sourceName,
  targetLabel,
} from './aboutYou';
import type { ProfileBuilderView, ProfileSuggestion } from './types';

const sug = (file: string, text: string, extra: Partial<ProfileSuggestion> = {}): ProfileSuggestion => ({
  id: text, file, label: null, section: null, text, source: 'claude-memory', createdAt: '2026-09-27T00:00:00Z', ...extra,
});

const view = (over: Partial<ProfileBuilderView> = {}): ProfileBuilderView => ({
  suggestions: [], questions: [], sources: [], profile: [], scannedAt: null, scanning: false, lastError: null,
  autoLearn: true, ...over,
});

describe('about you helpers', () => {
  it('names sources and counts them', () => {
    expect(sourceName('conversations')).toBe('your recent requests');
    expect(sourceName('bogus')).toBe('your agents');
    expect(sourceChip({ id: 'claude-memory', label: 'Claude memories', count: 1 })).toBe('1 Claude memory');
    expect(sourceChip({ id: 'conversations', label: 'Recent requests', count: 40 })).toBe('40 recent requests');
    expect(sourceChip({ id: 'codex-instructions', label: 'Codex instructions', count: 1 })).toBe('Codex instructions');
  });

  it('labels where a fact lands', () => {
    expect(targetLabel({ file: 'Identity.md', section: null, label: 'Role' })).toBe('Identity · Role');
    expect(targetLabel({ file: 'Communication.md', section: 'Always' })).toBe('Communication › Always');
    expect(targetLabel({ file: 'Guardrails.md', section: null })).toBe('Guardrails');
  });

  it('groups suggestions in profile order', () => {
    const groups = groupSuggestions(
      [sug('Communication.md', 'a'), sug('Identity.md', 'b'), sug('Communication.md', 'c')],
      ['Identity.md', 'Communication.md'],
    );
    expect(groups.map(g => [g.title, g.items.map(i => i.text)])).toEqual([['Identity', ['b']], ['Communication', ['a', 'c']]]);
  });

  it('says when it last looked', () => {
    const now = Date.parse('2026-09-27T12:00:00Z');
    expect(lastLooked(null, now)).toBeNull();
    expect(lastLooked('2026-09-27T11:59:50Z', now)).toBe('just now');
    expect(lastLooked('2026-09-27T11:30:00Z', now)).toBe('30m ago');
    expect(lastLooked('2026-09-27T09:00:00Z', now)).toBe('3h ago');
    expect(lastLooked('2026-09-26T12:00:00Z', now)).toBe('1d ago');
    expect(lastLooked('2026-09-27T13:00:00Z', now)).toBe('in 1h');
    expect(lastLooked('not a date', now)).toBeNull();
  });

  it('reads on its own only when never read or stale', () => {
    const now = Date.parse('2026-09-27T12:00:00Z');
    expect(shouldAutoScan(view(), now)).toBe(true);
    expect(shouldAutoScan(view({ scanning: true }), now)).toBe(false);
    expect(shouldAutoScan(view({ lastError: 'x' }), now)).toBe(false);
    expect(shouldAutoScan(view({ scannedAt: new Date(now - 1000).toISOString() }), now)).toBe(false);
    expect(shouldAutoScan(view({ scannedAt: new Date(now - AUTO_SCAN_AFTER_MS - 1).toISOString() }), now)).toBe(true);
  });

  it('never reads on its own with Keep learning off (P0-11)', () => {
    const now = Date.parse('2026-10-23T00:00:00Z');
    const fourDaysAgo = new Date(now - 4 * 24 * 3_600_000).toISOString();
    expect(shouldAutoScan(view({ autoLearn: false, scannedAt: fourDaysAgo }), now)).toBe(false);
    expect(shouldAutoScan(view({ autoLearn: false }), now)).toBe(false);
    expect(shouldAutoScan(view({ autoLearn: true, scannedAt: fourDaysAgo }), now)).toBe(true);
  });

  it('summarizes the profile', () => {
    const profile = [
      { path: 'me/Identity.md', title: 'Identity', exists: true, items: ['Name: J'], blanks: 2 },
      { path: 'me/Decisions.md', title: 'Decisions', exists: false, items: [], blanks: 0 },
    ];
    expect(completeness(profile)).toEqual({ filled: 1, blanks: 2, missing: 1 });
    expect(headline(view({ profile, suggestions: [sug('Identity.md', 'x')] }))).toBe('1 detail on file · 2 blanks to fill · 1 ready to add');
    expect(headline(view())).toMatch(/don’t know much/);
  });
});
