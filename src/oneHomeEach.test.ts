import { describe, expect, it } from 'vitest';
import connectorRowsSource from './settings/ConnectorRows.tsx?raw';
import settingsSource from './pages/Settings.tsx?raw';
import mainAgentSource from './settings/MainAgentSettings.tsx?raw';
import windowsPanelSource from './pages/workflows/AutomationWindowsPanel.tsx?raw';
import prioritizerSource from './settings/ContextPrioritizer.tsx?raw';
import agentDefaultsSource from './settings/AgentDefaults.tsx?raw';

// Settings PR 7 (one home each): the copies the plan listed under 2.3
// Duplicates are gone, and what is left names where the one home is. These
// quote source, like the other settings pins; the click paths are in the
// smoke specs (plaud-accesses, spending-caps, settings-sections).
const count = (text: string, part: string) => text.split(part).length - 1;

describe('one home each (Settings PR 7)', () => {
  it("the Plaud row in Connections no longer repeats the agents' connector state", () => {
    expect(connectorRowsSource).not.toContain('Plaud tools for agents:');
    expect(connectorRowsSource).not.toContain('props.agents');
    expect(settingsSource).toContain('<PlaudConnectorRow />');
  });

  it('the heartbeat on/off chip appears once, on Background work', () => {
    expect(count(settingsSource, "heartbeatEnabled() ? 'ok' : 'off'")).toBe(1);
  });

  it('the COO and Automation rows are links to Usage with no cap value of their own', () => {
    expect(mainAgentSource).not.toContain('cur().dailyCapUsd}</span>');
    expect(count(mainAgentSource, "requestOpenSettingsSection('usage', 'COO daily hard stop')")).toBe(1);
    expect(windowsPanelSource).not.toContain('capNote(');
    expect(count(windowsPanelSource, "openSettingsFromOutside('usage', 'Overnight cap')")).toBe(1);
  });

  it('both Jev switches say they need a key and link to the Jev row in Accounts', () => {
    expect(count(prioritizerSource, 'Needs a Jev key:')).toBe(2);
    expect(count(prioritizerSource, "requestOpenSettingsSection('accounts', 'Jev (TypeSafe)')")).toBe(3);
    expect(prioritizerSource).not.toContain('Add a key first');
  });

  it('the learnings frequency control carries its new label', () => {
    expect(agentDefaultsSource).toContain('<div class="settings-row-k">How many learnings to suggest</div>');
    expect(agentDefaultsSource).not.toContain('<div class="settings-row-k">Learning suggestions</div>');
  });
});
