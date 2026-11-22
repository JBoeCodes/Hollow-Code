import { beforeEach, describe, expect, it, vi } from 'vitest';

const calls = vi.hoisted(() => ({ tab: vi.fn(), windows: vi.fn(), page: vi.fn() }));
vi.mock('./libraryMode', () => ({ openLibraryTab: calls.tab }));
vi.mock('./automationsViews', () => ({ openAutomationWindows: calls.windows }));
vi.mock('./runtime', () => ({ setActivePage: calls.page }));

import runtimeSource from './runtime.ts?raw';
import navigationSource from '../settings/controllers/settingsNavigation.ts?raw';
import { openSettingsPageTarget } from './settingsPageTarget';
import { resolveSettingsTarget } from '../settingsResolve';

beforeEach(() => { calls.tab.mockClear(); calls.windows.mockClear(); calls.page.mockClear(); });

/** What `requestOpenSettingsSection(id, label)` does for a page target: resolve, then open. */
async function deepLink(id: string, label?: string) {
  const target = resolveSettingsTarget(id, label);
  if (target.kind !== 'page') throw new Error(`${id}/${label} is a section`);
  await openSettingsPageTarget(target);
}

describe('deep links to the pages Settings controls moved to (Settings PR 4)', () => {
  it.each([
    ['skills', undefined, 'skills'],
    ['connectors', 'Skills', 'skills'],
    ['history', undefined, 'history'],
    ['system', 'History & recovery', 'history'],
    ['operator', 'Ready to add', 'learnings'],
    ['system', 'Conversation history', 'conversations'],
  ])('%s / %s opens Library, %s', async (id, label, tab) => {
    await deepLink(id, label);
    expect(calls.tab).toHaveBeenCalledWith(tab, expect.any(Function));
    expect(calls.windows).not.toHaveBeenCalled();
  });

  it.each([['Automation windows', 'windows'], ['Run queue', 'queue']])('%s opens Automations on %s', async (label, tab) => {
    await deepLink('automation', label);
    expect(calls.windows).toHaveBeenCalledWith(tab, expect.any(Function));
    expect(calls.tab).not.toHaveBeenCalled();
  });

  it('opens a bare page when no tab is named', async () => {
    await openSettingsPageTarget({ page: 'workflows' });
    expect(calls.page).toHaveBeenCalledWith('workflows');
  });

  it('is what requestOpenSettingsSection, openSkills and Settings’ own jump end in', () => {
    expect(runtimeSource).toMatch(/if \(activePage\(\) !== 'settings'\) \{ void openSettingsPageTarget\(target\); return; \}/);
    expect(runtimeSource).toMatch(/export function openSkills\(\) \{\s*void openSettingsPageTarget\(\{ page: 'vault', tab: 'skills' \}\);/);
    // Settings' own jump asks about unsaved changes first (#1317 re-review).
    expect(navigationSource).toMatch(/if \(target\.kind === 'page'\) \{ openPageTarget\(target\); return; \}/);
    expect(navigationSource).not.toMatch(/void openSettingsPageTarget\(target\); return;/);
  });
});

describe('whether a moved target opened (review of fe901dc8f)', () => {
  it('resolves true once the destination commits, false when a guard refuses', async () => {
    calls.tab.mockImplementationOnce(async (_t: string, onOpen: () => void) => { onOpen(); });
    expect(await openSettingsPageTarget({ page: 'vault', tab: 'history' })).toBe(true);
    calls.tab.mockImplementationOnce(async () => undefined);
    expect(await openSettingsPageTarget({ page: 'vault', tab: 'history' })).toBe(false);
    calls.windows.mockImplementationOnce(async () => undefined);
    expect(await openSettingsPageTarget({ page: 'workflows', tab: 'queue' })).toBe(false);
  });
});
