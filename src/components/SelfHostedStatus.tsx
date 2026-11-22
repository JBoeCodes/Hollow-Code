/**
 * Self-hosted models Q6: the start chip above a self-hosted chat's composer
 * (lazy; `ChatBody` mounts it only while the chat's model is
 * `self-hosted:<entry id>`). Idle says the model sleeps, with Start now;
 * "Checking" for the first 3 s of a wake, then starting with about
 * 4 minutes and the time so far, as text; "<name> is ready" once its health
 * check answered; or the error in plain words. Its state and clock are
 * `selfHostedWake.ts` `createStatusView`: mounting reads the saved entries
 * (a local file) and the backend's last wake (memory), never a wake. Only
 * typing or sending in the composer (`store/selfHostedWake.ts`), or Start
 * now, wakes the model. The dot pulses only while starting, only with both
 * motion gates open, and pauses while the window is away
 * (`styles/self-hosted-status.css`).
 */
import { Show } from 'solid-js';
import { createStatusView } from '../selfHostedWake';
import { loadSelfHostedEntries, selfHostedEntry, selfHostedWakes } from '../store/selfHostedWake';
import '../styles/self-hosted-status.css';

export function SelfHostedStatus(props: { entryId: string }) {
  const name = () => selfHostedEntry(props.entryId)?.name ?? props.entryId;
  const chip = createStatusView({ entryId: () => props.entryId, name, wakes: selfHostedWakes, load: loadSelfHostedEntries });

  return (
    <div class="self-hosted-status" data-state={chip().kind} title={chip().title}>
      <i class="self-hosted-status-dot" aria-hidden="true" />
      <span class="self-hosted-status-text" role="status">{chip().text}</span>
      <Show when={chip().elapsed}>{text => <span class="self-hosted-status-elapsed">{text()}</span>}</Show>
      <Show when={chip().note}>{text => <span class="self-hosted-status-note">{text()}</span>}</Show>
      <Show when={chip().startNow !== 'hide'}>
        <button
          type="button"
          class="btn ghost sm"
          disabled={chip().startNow === 'wait'}
          title={chip().startNow === 'wait' ? chip().note : `Start ${name()} now; a start takes 4 to 6 minutes`}
          onClick={() => selfHostedWakes.startNow(props.entryId)}
        >Start now</button>
      </Show>
    </div>
  );
}
