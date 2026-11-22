// What the COO page's header band offered in its "…" menu, now in the top
// bar's "Actions for COO" menu (the band itself was removed 2026-11-11: it
// only took screen space). Every item keeps one visible click path; its
// facts live where they belong: the day's spend and check-ins in Settings,
// Automation → COO, subagents in the Subagents chip and tab.
import { api } from './api';
import { flashFooter, openVaultNote, openSettingsFromOutside } from './store/runtime';
import { openSessionByKey, refreshWorkspace, workspaceDetail, workspaceId } from './store/workspaces';
import { mainAgentActs, mainAgentActsRead, mainAgentNow, mainAgentStatus } from './store/mainAgent';
// Loaded on click: its import chain builds resources that need a browser.
const views = () => import('./store/automationsViews');
import { STANDING_ORDERS_FILE, checkInsOff, recentActionsFact } from './mainAgentSpace';
import type { ContextMenuItem } from './store';

let creatingOrders = false;
async function createOrders() {
  if (creatingOrders) return;
  creatingOrders = true;
  const owner = workspaceId();
  try {
    const created = await api.createMainStandingOrders();
    if (workspaceId() === owner) await refreshWorkspace();
    flashFooter(created ? 'Default standing orders created.' : 'Standing orders already exist; kept your file.');
  } catch (error) { flashFooter(`Could not create standing orders: ${String(error)}`, 5000); }
  finally { creatingOrders = false; }
}

export function cooMenuItems(): ContextMenuItem[] {
  const d = workspaceDetail();
  const items: ContextMenuItem[] = [
    { label: `Actions it took on its own · ${recentActionsFact(mainAgentActs().length, mainAgentActsRead())}`, action: () => void views().then(v => v.openMainAgentActions()) },
    { label: 'Change hard stop', action: () => { openSettingsFromOutside('usage', 'COO daily hard stop'); } },
    { label: checkInsOff(mainAgentStatus()) ? 'Turn check-ins on in Settings' : 'Check-in settings', action: () => { openSettingsFromOutside('automation', 'COO check-ins'); } },
    { label: 'How check-ins run', action: () => void views().then(v => v.openBackgroundWork()) },
  ];
  const failed = mainAgentNow().failedSession;
  if (failed) items.push({ label: 'Open failed conversation', action: () => void openSessionByKey(failed) });
  items.push({ separator: true });
  if (d && !d.files.some(f => f.name === STANDING_ORDERS_FILE)) items.push({ label: 'Create default standing orders', action: () => void createOrders() });
  else if (d) items.push({ label: 'Edit standing orders', action: () => openVaultNote(`${d.workspace.path}/${STANDING_ORDERS_FILE}`) });
  return items;
}
