// The composer's access-mode chip (overnight 11-12, C3): the chat's
// permission mode as words beside the +, warning-coloured in a danger mode,
// opening the same choices as the model menu's Permissions row. Since
// 2026-11-12 it names the chat's agent's own modes (`agentPermModes.ts`:
// Claude's Accept edits, Codex's Ask for approval, Antigravity's Skip
// permissions). A change asks first (confirmPermissionModeChange) like every
// mode picker and waits while a turn runs.
import { ActionMenu } from './ActionMenu';
import { Icon } from '../icons';
import { accessChipName, accessMenuItems } from '../composerBar';
import { permModeNote, permModeOptions, permModeUnavailable } from '../permModes';
import { agentModeChips, agentModeDanger, agentModeDescriptions, agentModeLabels, agentModeOrder, hollowCardsLine, shownMode } from '../agentPermModes';
import {
  chatAgent, chatBusy, chatSessionLoading, chatPermissionMode, setChatPermissionModePersistent,
  confirmPermissionModeChange, currentAgentCapabilities, sameChatAsNow, type ChatPermissionMode,
} from '../store';
import '../styles/composer-commands.css';

export function ComposerAccessChip() {
  // The stored mode as this agent's option shows it (a Codex chat stored as
  // acceptEdits reads Ask for approval, which runs the same way).
  const mode = () => shownMode(chatAgent(), chatPermissionMode());
  // The chip says the short name; its menu lists the agent's full names.
  const chips = () => agentModeChips(chatAgent());
  const labels = () => agentModeLabels(chatAgent());
  const danger = () => agentModeDanger(chatAgent())[mode()];
  const locked = () => chatBusy() || chatSessionLoading();
  async function pick(next: ChatPermissionMode) {
    if (next === mode() || locked() || permModeUnavailable(currentAgentCapabilities(), next)) return;
    // The confirm's answer belongs to the chat it was asked on.
    const still = sameChatAsNow();
    if (!(await confirmPermissionModeChange(next, chatAgent())) || locked() || !still()) return;
    setChatPermissionModePersistent(next);
  }
  const items = () => accessMenuItems({
    options: permModeOptions(agentModeOrder(chatAgent()), labels(), agentModeDescriptions(chatAgent()), currentAgentCapabilities()),
    current: mode(),
    danger: agentModeDanger(chatAgent()),
    locked: locked(),
    note: permModeNote(labels()[mode()], agentModeDescriptions(chatAgent())[mode()], currentAgentCapabilities(), mode()),
    footer: hollowCardsLine(chatAgent(), currentAgentCapabilities()?.ownedMcp),
    onPick: next => void pick(next),
  });
  return (
    <span class="composer-access" classList={{ danger: danger() }}>
      <ActionMenu label={accessChipName(chips()[mode()], danger())} align="start" items={items()}
        icon={<><Icon name={danger() ? 'alert' : 'shieldCheck'} size={14} /><span class="composer-access-label">{chips()[mode()]}</span></>} />
    </span>
  );
}
