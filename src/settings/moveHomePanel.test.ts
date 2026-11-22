import { beforeEach, describe, expect, it, vi } from 'vitest';
import panel from './MoveHomePanel.tsx?raw';
import settingsPage from '../pages/Settings.tsx?raw';
import appSource from '../App.tsx?raw';
import { mockMoveHomeCommand } from '../mockMoveHome';
import { planSections, rollbackState, startBlockedReason } from '../moveHome';
import type { MoveHomePlan, MoveHomeStatus } from '../types';

// Settings, Sources, Hollow folder, Move Hollow home (2026-11-12). Vitest
// here has no DOM or Solid compiler, so the panel's decisions live in
// moveHome.ts and are driven through the browser mock below, and this file
// pins how the panel wires them; tests/smoke/move-home.spec.ts clicks it.

const storage = new Map<string, string>();
beforeEach(() => {
  storage.clear();
  vi.stubGlobal('localStorage', { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => { storage.set(k, v); }, removeItem: (k: string) => { storage.delete(k); } });
});
const plan = (options = {}) => mockMoveHomeCommand('move_home_plan', { options }) as MoveHomePlan;
const status = () => mockMoveHomeCommand('move_home_status', {}) as MoveHomeStatus;

describe('Move Hollow home panel', () => {
  it('is its own lazy chunk under the Hollow folder, kept out of the shell', () => {
    expect(settingsPage).toContain("const MoveHomePanel = lazy(() => import('../settings/MoveHomePanel')");
    expect(settingsPage).not.toMatch(/import \{[^}]*\} from '\.\.\/settings\/MoveHomePanel'/);
    const part = settingsPage.slice(settingsPage.indexOf('id="vault"'), settingsPage.indexOf('</SettingsPart>', settingsPage.indexOf('id="vault"')));
    expect(part).toContain('<DirectorySetup />');
    expect(part).toContain('<SettingsGroup id="move-home" title="Move Hollow home"');
    expect(part).toContain('<MoveHomePanel />');
    expect(appSource).not.toMatch(/moveHome|MoveHomePanel/);
    // Its requests come from moveHomeApi.ts, not api.ts's shell object.
    expect(panel).toContain("from '../moveHomeApi'");
    expect(panel).not.toMatch(/from '\.\.\/(api|store)/);
  });

  it('every capability is a visible button: check, the option, quit and move with a second confirm, roll back', () => {
    for (const label of ["'Check the move'", 'Also move the folders beside the vault', 'Quit and move…', 'Yes, quit and move', 'Roll back…', 'Yes, roll back'])
      expect(panel).toContain(label);
    expect(panel).toContain('includeVaultSiblings: siblings()');
    expect(panel).toContain('moveHomeStart(p.hash, options())');
  });

  it('the dry run shows the report: each section of the mock plan has a heading and lines', () => {
    expect(panel).toContain('<For each={planSections(p())}>');
    const sections = planSections(plan());
    expect(sections.length).toBeGreaterThan(8);
    for (const s of sections) expect(s.lines.length).toBeGreaterThan(0);
  });

  it('Quit and move is disabled with its reason while a chat runs', () => {
    expect(panel).toMatch(/aria-disabled=\{!!busy\(\) \|\| !!blockedReason\(\)\}/);
    expect(panel).toContain('id="move-home-blocked"');
    expect(startBlockedReason(plan(), status())).toBe('');
    storage.set('mockMoveHomeChatRunning', '1');
    expect(startBlockedReason(plan(), status())).toMatch(/^Wait for 1 chat to finish/);
  });

  it('Roll back shows once a move exists, with the refusal when there is one', () => {
    expect(panel).toContain('<Show when={rollback().shown}>');
    expect(panel).toContain('id="move-home-rollback-refused"');
    expect(rollbackState(status()).shown).toBe(false);
    storage.set('mockMoveHomeState', 'done');
    expect(rollbackState(status())).toEqual({ shown: true, disabledReason: '' });
    storage.set('mockMoveHomeRefuse', '1');
    expect(rollbackState(status()).disabledReason).toMatch(/Copy them out/);
  });

  it('polls the journal only while a move is staged or running, and stops on unmount', () => {
    expect(panel).toContain('if (shouldPoll(next)) timer = setTimeout(');
    expect(panel).toContain('onCleanup(() => { alive = false; clearTimeout(timer); });');
  });

  it('writes no em or en dashes', () => {
    expect(panel).not.toMatch(/[\u2013\u2014]/);
  });
});
