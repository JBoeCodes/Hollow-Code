import { describe, expect, it, vi } from 'vitest';

vi.mock('./runtime', () => ({ setActivePage: vi.fn() }));

import runtimeSource from './runtime.ts?raw';
import navigationSource from '../settings/controllers/settingsNavigation.ts?raw';
import bridgeSource from '../appBridge.ts?raw';
import { directPageTarget } from './settingsPageTarget';

// A link into Settings from another page (review of #1337): a moved control
// opens on its own page with no Settings step in Back.
describe('settings: links and ui.navigate into a moved control', () => {
  it('opens a page target directly from another page', () => {
    expect(directPageTarget('history', undefined, false)).toMatchObject({ page: 'vault', tab: 'history' });
    expect(directPageTarget('skills', '', false)).toMatchObject({ page: 'vault', tab: 'skills' });
  });

  it('goes through Settings for a section, or when Settings is showing', () => {
    expect(directPageTarget('heartbeat', undefined, false)).toBeNull();
    expect(directPageTarget('history', undefined, true)).toBeNull();
  });

  it('is what the settings: URL and ui.navigate both use', () => {
    expect(runtimeSource).toContain('    openSettingsFromOutside(section);');
    expect(runtimeSource).toContain("directPageTarget(id, control, activePage() === 'settings')");
    expect(bridgeSource).toContain('const { target: t, opened, direct } = openSettingsFromOutside(target);');
    expect(bridgeSource).toContain("visiblePath(t.page === 'vault' ? 'library' : 'automations'");
  });

  it('with Settings showing, a page target is published to Settings, whose jump asks; a refused open is not answered as opened', () => {
    expect(runtimeSource).toContain("if (activePage() !== 'settings') { void openSettingsPageTarget(target); return; }");
    expect(navigationSource).toContain("if (target.kind === 'page' || flat.some(s => s.id === id)) jumpTo(req.id, req.control);");
    expect(bridgeSource).toContain('if (!(await opened)) throw new Error(');
    expect(bridgeSource).toContain(`if (!(await opened)) throw new Error("navigate: settings didn't open (refused or replaced by another navigation).");`);
  });

  it("the composer's Usage details opens Settings, not only a section request", () => {
    expect(sources['../components/ChatUsageMenu.tsx']).toContain("onClick={() => { setOpen(false); openSettingsFromOutside('usage'); }}");
  });
});

// Overnight 11-13 S3: the other deep-link callers go the same way, so a
// moved control (Library, Automations) never leaves a Settings step in Back.
// Overnight 11-13 S6 (and its review): every link into Settings from another
// page goes through openSettingsFromOutside, so an unsaved-draft refusal
// leaves no section request pending. A scan, so a new outside caller fails.
const sources = import.meta.glob(['../**/*.ts', '../**/*.tsx', '!../**/*.test.ts', '!../**/*.test.tsx'],
  { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
/** Settings itself (its own jump asks about unsaved changes), the definition,
 *  and agentHealth, which wraps its page change and request in guardNavigation. */
const MAY_REQUEST = (file: string) => file.startsWith('../settings/') || /^(\.\.\/store|\.)\/(runtime|agentHealth)\.ts$/.test(file);

describe('links into Settings from other pages', () => {
  it('open Settings through openSettingsFromOutside, never by switching the page and requesting a section', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(100);
    const outside = Object.entries(sources).filter(([file, text]) => !MAY_REQUEST(file) && text.includes('requestOpenSettingsSection('));
    expect(outside.map(([file]) => file)).toEqual([]);
    for (const file of ['../components/ChatUsageMenu.tsx', '../components/ShortcutsOverlay.tsx', '../components/SemanticSearchLink.tsx', '../cooMenu.ts', '../actionRun.ts'])
      expect(sources[file], file).toContain('openSettingsFromOutside');
  });
});
