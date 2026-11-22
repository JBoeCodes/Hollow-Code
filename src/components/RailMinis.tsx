import { For, Show } from 'solid-js';
import { activePage, canLeaveConversation } from '../store';
import { workspaceId, openProjectAgent } from '../store/workspaces';
import { workspaceRollup } from '../store/sessionStates';
import type { WorkWorkspace } from '../workspaceTypes';
import { morphFrom } from '../viewTransitions';
import { ProjectIcon } from './ProjectIcon';
import { SessionRollupBadge } from './SessionIndicator';

/** Loaded only for the collapsed rail, inside its own recovery/loading
 *  boundary. Each pinned project's icon opens its lead, as its row does
 *  (L8, 11-10); chats with no project go to the COO, so there is no Chats
 *  icon any more. */
export default function RailMinis(props: { pinned: WorkWorkspace[] }) {
  return <Show when={props.pinned.length}>
    <div class="work-rail-pinned-mini" role="group" aria-label="Pinned workspaces"><For each={props.pinned}>{w=>
      <button data-key-item class="work-rail-pinned-mini-row" classList={{selected:activePage()==='today'&&workspaceId()===w.id}} aria-label={[w.title,'pinned',workspaceRollup(w.id)?.label].filter(Boolean).join(', ')} title={[w.title,workspaceRollup(w.id)?.label].filter(Boolean).join(', ')} disabled={!canLeaveConversation()&&workspaceId()!==w.id} onClick={e=>{morphFrom(e.currentTarget);void openProjectAgent(w.id);}}><ProjectIcon workspaceId={w.id}/><SessionRollupBadge rollup={workspaceRollup(w.id)}/></button>}
    </For></div>
  </Show>;
}
