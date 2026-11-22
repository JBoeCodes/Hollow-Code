import { Show, createResource } from 'solid-js';
import { Icon } from '../icons';
import { api } from '../api';
import { ActionMenu } from './ActionMenu';
import { MachineGlyph } from './MachineBadge';
import { peerMachines, loadPeerMachines } from '../store/peerSessions';
import { chatAgentSessionKey, chatCwd } from '../store/chatRuntime';
import { DRAFT_TAB_OWNER, openPreview } from '../store/preview';
import { setScriptEditor } from '../store/projectScripts';
import { flashFooter } from '../store/runtime';
import { errMsg } from '../utils';
import { cardMachineName, openInRows } from '../sidePanelModel';

/** The card's top rows (parity 5.1): the machine this chat's folder is on,
 *  Open in (Finder and the editors installed here) and Add project script.
 *  Every row is a visible button or menu item; none needs a chord. */
export function SidePanelCardHeader(props: { project: boolean; workspaceId: string | null }) {
  const [targets] = createResource(async () => { try { return await api.openInTargets(); } catch { return []; } });
  // This Mac's name arrives with the machine list the rail already reads.
  if (!peerMachines()) void loadPeerMachines(false);
  const machine = () => cardMachineName(peerMachines()?.thisMac?.name);
  const openIn = (id: string, label: string) => {
    const cwd = chatCwd();
    if (!cwd) return;
    void api.openInOpen(cwd, id).catch(e => flashFooter(`× Couldn't open it in ${label}: ${errMsg(e)}`, 5000));
  };
  return (
    <div class="side-card-header">
      <p class="side-card-machine" title={`This chat's folder is on ${machine()}`} aria-label={`Machine: ${machine()}`}>
        <MachineGlyph size={13} /><span>{machine()}</span>
      </p>
      <div class="side-card-header-actions">
        <Show when={chatCwd()}>
          <ActionMenu label="Open in" align="start" icon={<><Icon name="folder" size={14} /><span class="side-card-open-in">Open in</span></>}
            items={openInRows(targets()).map(t => ({ label: t.label, title: `Open this chat's folder in ${t.label}`, action: () => openIn(t.id, t.label) }))} />
        </Show>
        <button type="button" class="side-card-script" title="Open a terminal in this chat's folder, where the chat's mode allows one (parity 7.2)"
          onClick={() => openPreview({ kind: 'terminal', chat: chatAgentSessionKey() ?? DRAFT_TAB_OWNER })}><Icon name="terminal" size={14} /><span>Terminal</span></button>
        <Show when={props.project && props.workspaceId}>
          <button type="button" class="side-card-script" title="Add a project script: a named command you run from the card (parity 4.5)"
            onClick={() => setScriptEditor({ workspaceId: props.workspaceId!, scriptId: null })}><Icon name="plus" size={14} /><span>Add project script</span></button>
        </Show>
      </div>
    </div>
  );
}
