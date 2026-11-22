// The side panel's Computer section (Cua CU8): what the agent last saw on the
// desktop, its recent actions with their tier, and the person's controls:
// Pause, Take over, Hand back and Stop. Read from the broker's per-chat state
// (`computer_chat_state`), polled while the section is open.
import { For, Show } from 'solid-js';
import { chatUsesComputer, controlComputer, useComputerChat } from '../store/computerChat';
import { actionWords } from '../computerWords';

export function ComputerCard(props: { open: () => boolean }) {
  const { view, frame } = useComputerChat(() => chatUsesComputer() && props.open());
  const v = () => view();
  return (
    <div class="computer-card">
      <Show when={frame()} fallback={<p class="side-card-empty">No picture yet.</p>}>
        <img class="computer-frame" src={frame()!} alt="What the agent last saw on the desktop" />
      </Show>
      <p class="side-card-note" role="status">
        {v()?.takenOver ? 'You have the controls. The agent is waiting.' : v()?.paused ? 'Paused. The agent is waiting.' : 'The agent can see and act on the desktop.'}
      </p>
      <div class="computer-controls">
        <Show when={v()?.takenOver} fallback={<>
          <button type="button" class="btn small" onClick={() => void controlComputer(v()?.paused ? 'resume' : 'pause')}>{v()?.paused ? 'Resume' : 'Pause'}</button>
          <button type="button" class="btn small" onClick={() => void controlComputer('takeover')}>Take over</button>
        </>}>
          <button type="button" class="btn small" onClick={() => void controlComputer('handback')}>Hand back</button>
        </Show>
        <button type="button" class="btn small danger" onClick={() => void controlComputer('stop')}>Stop</button>
      </div>
      <ul class="computer-actions" aria-label="Recent desktop actions">
        <For each={(v()?.entries ?? []).slice(0, 8)} fallback={<li class="side-card-empty">No actions yet.</li>}>
          {e => <li data-outcome={e.outcome}>{actionWords(e)}</li>}
        </For>
      </ul>
    </div>
  );
}
