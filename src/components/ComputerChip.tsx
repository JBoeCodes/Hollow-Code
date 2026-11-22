// The composer's desktop chip (Cua CU8): shown once this chat has used the
// desktop tools, beside the access chip. It says whether the desktop is
// running, paused or in the person's hands, and its menu holds the controls
// (Pause or Resume, Take over or Hand back, Stop) and a way to the Computer
// control settings. The same controls are on the side-panel Computer card.
import { Show } from 'solid-js';
import { ActionMenu, type ActionMenuItem } from './ActionMenu';
import { Icon } from '../icons';
import { chatUsesComputer, controlComputer, useComputerChat } from '../store/computerChat';
import { openSettingsFromOutside } from '../store/runtime';

export function ComputerChip() {
  const { view } = useComputerChat(chatUsesComputer);
  const held = () => view()?.paused ?? false;
  const word = () => view()?.takenOver ? 'In your hands' : held() ? 'Paused' : 'Desktop';
  const items = (): ActionMenuItem[] => {
    const v = view();
    const list: ActionMenuItem[] = [
      { label: v?.takenOver ? 'Desktop: you have the controls' : held() ? 'Desktop: paused, the agent is waiting' : 'Desktop: the agent can see and act here', action: () => {}, informational: true },
    ];
    if (v?.takenOver) list.push({ label: 'Hand back to the agent', action: () => void controlComputer('handback') });
    else {
      list.push(held()
        ? { label: 'Resume', action: () => void controlComputer('resume') }
        : { label: 'Pause', action: () => void controlComputer('pause') });
      list.push({ label: 'Take over', action: () => void controlComputer('takeover'), title: 'Pause the agent so you can use the desktop yourself' });
    }
    list.push({ label: 'Stop computer control in this chat', action: () => void controlComputer('stop'), danger: true, separatorBefore: true });
    list.push({ label: 'Computer control settings', action: () => openSettingsFromOutside('connectors', 'Computer control') });
    return list;
  };
  return (
    <Show when={chatUsesComputer() || held()}>
      <span class="composer-access" classList={{ danger: held() }}>
        <ActionMenu label={`Computer control: ${word()}`} align="start" items={items()}
          icon={<><Icon name="appWindow" size={14} /><span class="composer-access-label">{word()}</span></>} />
      </span>
    </Show>
  );
}
