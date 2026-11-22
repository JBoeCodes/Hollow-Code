// Shared visible recovery route and installed-version invalidation. These
// never update an agent; the person chooses that in Agent updates.
import type { AgentCliStatus } from '../generated/ipc';
import { guardNavigation } from './navigationGuard';
import { goToPage } from './workspaces';
import { requestOpenSettingsSection } from './runtime';
import { ensureChatSteerSupport } from './chatRuntime';
import { flashWithUndo } from './snackbars';

export function openAgentUpdates(): void {
  void guardNavigation(() => {
    goToPage('settings');
    requestOpenSettingsSection('defaults', 'Agent updates');
  });
}

export function unavailableAgentNotice(message: string): void {
  flashWithUndo({ message, kind: 'error', ttlMs: 8000,
    actions: [{ label: 'Agent updates', run: openAgentUpdates }] });
}

let installedIdentity = '';
export function syncAgentHealthSupport(rows: AgentCliStatus[]): void {
  const identity = rows.map(row => [row.agent, row.installed, row.version, row.state].join(':')).sort().join('|');
  const changed = identity !== installedIdentity;
  installedIdentity = identity;
  ensureChatSteerSupport(changed);
}
