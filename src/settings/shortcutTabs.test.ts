import { describe, expect, it } from 'vitest';
import { SHORTCUT_TABS } from './shortcutTabs';

describe('keyboard shortcut tabs', () => {
  it('has Commands first, then Go to keys', () => {
    expect(SHORTCUT_TABS.map(t => t.label)).toEqual(['Commands', 'Go to keys']);
  });
});
