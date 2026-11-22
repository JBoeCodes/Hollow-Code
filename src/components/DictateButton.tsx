// The microphone in a composer (overnight 10-11 B7): click to dictate into
// that composer's box, click again to stop. On-device only; the words land
// in the box and are never sent on their own. ⌘⇧D does the same for the
// focused field (`store/dictation.ts`).
// P1-55: the name stays "Dictate" (its state is aria-pressed), the state is
// said once in a live region beside the button, and when dictation is off
// the reason is the button's description.
import { Show, createUniqueId, onMount } from 'solid-js';
import { Icon } from '../icons';
import { activeChord, displayChord } from '../shortcuts';
import { statusTitle } from '../dictation';
import { dictationPhase, dictationStatus, dictationTarget, refreshDictationStatus, toggleDictation } from '../store/dictation';
import '../styles/dictation.css';

export function DictateButton(props: { target: () => import('../composerField').ComposerField | undefined }) {
  onMount(() => { if (!dictationStatus()) void refreshDictationStatus(); });
  const id = createUniqueId();
  const mine = () => !!props.target() && dictationTarget() === props.target();
  const live = () => mine() && dictationPhase() !== 'idle';
  const off = () => { const s = dictationStatus()?.state; return s === 'unavailable' || s === 'denied'; };
  const title = () => live() ? 'Stop dictating (click, Esc, or the dictate key again)' : statusTitle(dictationStatus(), displayChord(activeChord('dictate')));
  const word = () => dictationPhase() === 'starting' ? 'Starting' : dictationPhase() === 'stopping' ? 'Finishing' : 'Listening';
  return (
    <>
      <button
        type="button"
        class="chat-composer-btn dictate-btn"
        classList={{ 'is-live': live(), 'is-off': off() && !live() }}
        aria-pressed={live()}
        aria-label="Dictate"
        aria-describedby={off() && !live() ? `${id}-why` : undefined}
        title={title()}
        onMouseDown={e => e.preventDefault()}
        onClick={() => toggleDictation(props.target())}
      >
        <Icon name="mic" size={14} />
        <Show when={live()}>
          <span class="dictate-pill" aria-hidden="true">{word()}…</span>
        </Show>
      </button>
      <span class="sr-only dictate-status" role="status">{live() ? word() : ''}</span>
      <Show when={off() && !live()}><span class="sr-only" id={`${id}-why`}>{dictationStatus()?.reason ?? 'Dictation is unavailable.'}</span></Show>
    </>
  );
}
