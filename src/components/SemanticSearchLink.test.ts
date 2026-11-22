import { describe, expect, it, vi } from 'vitest';
import { SETTINGS_SECTION_BY_ID, resolveSettingsSection } from '../settingsSections';

// The component module imports the store, which touches localStorage at load.
vi.mock('../store', () => ({ openSettingsFromOutside: vi.fn(), semanticSearchEnabled: () => false }));

const { SEMANTIC_SEARCH_TARGET, openSemanticSearchSettings } = await import('./SemanticSearchLink');

describe('Library\'s "Turn on search by meaning" link', () => {
  it('targets the semantic search switch, a registered control in its section', () => {
    const section = resolveSettingsSection(SEMANTIC_SEARCH_TARGET.section);
    const labels = SETTINGS_SECTION_BY_ID[section].controls!.map(c => c.label);
    expect(section).toBe('advanced'); // the old `system` id, which still resolves
    expect(labels).toContain(SEMANTIC_SEARCH_TARGET.control);
    expect(SEMANTIC_SEARCH_TARGET.control).toBe('Index my vault for semantic search');
  });

  it('opens Settings on that control in one guarded jump', () => {
    const calls: string[] = [];
    openSemanticSearchSettings({ open: (s, c) => calls.push(`${s}:${c}`) });
    expect(calls).toEqual(['system:Index my vault for semantic search']);
  });
});
