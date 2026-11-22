// COO rebuild L6 (2026-11-10): "Main agent" is the COO everywhere a person
// or an agent reads it. Internal ids (`main`, `main-agent`, MAIN_AGENT_ID,
// MainAgent* modules) stay, so only words with a space are checked, and
// search keywords may keep the old name as an alias.
import { describe, expect, it } from 'vitest';
import { ROUTES, RIGHT_CLICK, renderActions, renderRouteLedger } from './routeLedger';
import { ACTION_DEFS, CONDITIONS, actionLabel } from './actionDefs';
import { SETTINGS_SECTION_GROUPS } from './settingsSections';
import { COMMANDS } from './shortcuts';
import { GO_TARGETS } from './keyboardLeader';
import { ASK_SCOPES } from './askScopes';
import { BACKGROUND_SYSTEMS } from './backgroundWork';
import { MAIN_AGENT_TITLE } from './workspaceTypes';
import settingsPage from './pages/Settings.tsx?raw';
import mainAgentSettings from './settings/MainAgentSettings.tsx?raw';
import customIconRows from './settings/CustomIconRows.tsx?raw';
import customIconEditor from './components/CustomIconEditor.tsx?raw';
import mainAgentOrb from './components/MainAgentOrb.tsx?raw';
import sessionIndicator from './components/SessionIndicator.tsx?raw';
import rail from './components/Rail.tsx?raw';
import chatModelMenu from './components/ChatModelMenu.tsx?raw';
import workspaceMenu from './workspaceMenu.ts?raw';
import conversationAbout from './conversationAbout.ts?raw';
import newWorkspaceForm from './components/NewWorkspaceForm.tsx?raw';
import appMenu from '../src-tauri/src/app_menu.rs?raw';
import trayLib from '../src-tauri/src/lib.rs?raw';
import chatRs from '../src-tauri/src/chat.rs?raw';

const OLD = /\bmain agent\b/i;

/** Every string in a value, without `keywords` (search aliases, not shown). */
function shown(value: unknown, out: string[] = []): string[] {
  if (typeof value === 'string') out.push(value);
  else if (Array.isArray(value)) value.forEach(v => shown(v, out));
  else if (value && typeof value === 'object')
    for (const [key, v] of Object.entries(value)) if (key !== 'keywords') shown(v, out);
  return out;
}

// A source file's code without its comments: line comments and block comments, JSX ones included.
const code = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '').replace(/[^:'"`]\/\/.*$/gm, '');

describe('the COO name (L6)', () => {
  it('route ledger rows, right-click rows and the generated ledgers never say Main agent', () => {
    for (const text of shown([ROUTES, RIGHT_CLICK])) expect(text).not.toMatch(OLD);
    expect(renderRouteLedger()).not.toMatch(OLD);
    expect(renderActions()).not.toMatch(OLD);
  });

  it('action labels, conditions and their surfaces say COO', () => {
    for (const text of shown([ACTION_DEFS, CONDITIONS])) expect(text).not.toMatch(OLD);
    for (const def of ACTION_DEFS)
      for (const surface of ['palette', 'key', 'menu'] as const) expect(actionLabel(def, surface)).not.toMatch(OLD);
    const main = ACTION_DEFS.find(a => a.id === 'main-agent')!;
    expect([main.label, actionLabel(main, 'palette')]).toEqual(['COO', 'Open the COO']);
  });

  it('settings sections, their titles, scopes and control labels say COO', () => {
    for (const text of shown(SETTINGS_SECTION_GROUPS)) expect(text).not.toMatch(OLD);
    const labels = SETTINGS_SECTION_GROUPS.flatMap(g => g.items.flatMap(s => (s.controls ?? []).map(c => c.label)));
    expect(labels).toContain('COO daily hard stop');
    expect(labels).toContain('COO icon');
  });

  it('shortcuts, Go keys, Ask scopes and background work say COO', () => {
    for (const text of shown([COMMANDS, GO_TARGETS, ASK_SCOPES, BACKGROUND_SYSTEMS])) expect(text).not.toMatch(OLD);
    expect(MAIN_AGENT_TITLE).toBe('COO');
  });

  it('the screens, the app menu and the tray say COO in their code', () => {
    const sources = { settingsPage, mainAgentSettings, customIconRows, customIconEditor, mainAgentOrb, sessionIndicator, rail, chatModelMenu, workspaceMenu, conversationAbout, newWorkspaceForm, appMenu, trayLib };
    for (const [name, source] of Object.entries(sources)) expect(code(source), name).not.toMatch(OLD);
    expect(appMenu).toContain('("main-drawer", "COO",');
    expect(code(settingsPage)).toContain('<SettingsSubhead title="COO"');
  });

  it('the COO is not Claude-only any more, and its prompt delegates by default', () => {
    expect(code(chatModelMenu)).not.toMatch(/Claude only|requires Claude/);
    const prompt = chatRs.slice(chatRs.indexOf('fn main_agent_prompt'), chatRs.indexOf('const MAIN_AGENT_PROMPT_BUDGET'));
    expect(prompt).toContain("You are Hollow's COO");
    expect(prompt).toContain('You delegate by default (a habit, not a tool limit');
    expect(chatRs).not.toContain('fn main_agent_provider');
  });
});
