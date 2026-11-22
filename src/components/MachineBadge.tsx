// M2 PR 4: the small badge after a remote chat's title (`docs/reference/
// peers.md`, "The UI"): a machine glyph and the Mac's short word ("mini"),
// named "On Mac mini" for screen readers, with a dot while the chat waits for
// an answer. Local chats have no badge. The badge shows the chat's home (which
// Hollow owns its record), never where its agent runs.
import { Show } from 'solid-js';
import type { MachineRef } from '../generated/ipc';
import { badgeLabel, machineWord } from './machineStrip';
import { movedLabel } from '../store/chatMoveWords';
import '../styles/peer-machines.css';

/** A laptop-and-desk glyph in the icon set's style (lazy chunks only, so the
 *  shell's icon table doesn't grow). */
export function MachineGlyph(props: { size?: number }) {
  const size = () => props.size ?? 12;
  return (
    <svg class="machine-glyph" xmlns="http://www.w3.org/2000/svg" width={size()} height={size()} viewBox="0 0 24 24" fill="none"
      stroke="currentColor" stroke-width={Math.min(2.5, Math.max(1.4, +(27.6 / size()).toFixed(2)))} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="4" y="5" width="16" height="11" rx="1.5" />
      <path d="M2 19h20" />
    </svg>
  );
}

/** M5: `moved` marks a chat that moved between Macs. `from`: the row at its
 *  new home ("mini · moved"). `to`: the row left on this Mac because its new
 *  home lists nothing now ("moved"; `machine` null for a Mac you forgot). */
export function MachineBadge(props: { machine: MachineRef | null; needsAnswer?: boolean; moved?: 'from' | 'to' }) {
  const label = () => props.moved === 'to' || !props.machine ? movedLabel(props.machine?.name ?? null)
    : badgeLabel(props.machine, !!props.needsAnswer) + (props.moved ? ', moved there from another Mac' : '');
  const word = () => props.moved === 'to' || !props.machine ? 'moved' : machineWord(props.machine) + (props.moved ? ' · moved' : '');
  return (
    <span class="machine-badge" classList={{ 'is-waiting': !!props.needsAnswer, 'is-moved': !!props.moved }} role="img" aria-label={label()} title={label()}>
      <MachineGlyph size={11} />
      <span class="machine-badge-word" aria-hidden="true">{word()}</span>
      <Show when={props.needsAnswer}><span class="machine-badge-dot" aria-hidden="true" /></Show>
    </span>
  );
}
