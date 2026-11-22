import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import type { AutomationWindows } from '../generated/ipc';
import { backgroundClock, clockArc, clockTime, type ClockBand } from '../backgroundClock';
import { openAutomationsView } from '../store/automationsViews';
import { ErrorText } from './ui';
import '../styles/background-clock.css';

export function BackgroundClock(props: {
  windows: AutomationWindows | null; loading: boolean; error: string; refresh: () => void;
}) {
  const [now, setNow] = createSignal(new Date());
  onMount(() => {
    const update = () => { if (!document.hidden) setNow(new Date()); };
    const timer = setInterval(update, 60_000);
    document.addEventListener('visibilitychange', update);
    onCleanup(() => { clearInterval(timer); document.removeEventListener('visibilitychange', update); });
  });
  const model = createMemo(() => backgroundClock(props.windows, now()));
  const [bands, setBands] = createStore<ClockBand[]>([]);
  createEffect(() => setBands(reconcile(model()?.bands ?? [], { key: 'name' })));
  const ranges = (band: ClockBand) => band.spans.length
    ? band.spans.map(s => `${clockTime(s.start)}–${clockTime(s.end)}${s.fromYesterday ? ' (from yesterday)' : ''}`).join(', ')
    : 'No window on this date';
  return <section class="background-clock" aria-label="Background work windows">
    <details>
      <summary>Background work windows<span role="status"><Show when={props.error}> · Could not read</Show><Show when={!props.error && props.loading}> · Reading…</Show></span></summary>
    <Show when={!props.loading} fallback={<p role="status">Reading background work windows…</p>}>
      <Show when={!props.error} fallback={<ErrorText class="background-clock-error" text={`Couldn’t read background work windows: ${props.error}`}><button class="btn-link" onClick={props.refresh}>Retry windows</button></ErrorText>}>
        <Show when={model()} fallback={<p>Background work windows are unknown. <button class="btn-link" onClick={props.refresh}>Retry windows</button></p>}>{m =>
          <div class="background-clock-layout">
            {/* Native legend buttons provide the same routes for keyboard/AT.
                The drawing is an extra pointer target, never the only route. */}
            <svg class="background-clock-dial" viewBox="0 0 160 160" aria-hidden="true">
              <circle cx="80" cy="80" r="57" class="background-clock-track" />
              <For each={bands.slice(0, 6)}>{(band, i) =>
                <g class="background-clock-band" data-window={band.name} onClick={() => openAutomationsView('automations')}>
                  <For each={band.spans}>{span => <path d={clockArc(span.start, span.end, 56 - i() * 4)} />}</For>
                </g>
              }</For>
              <text x="80" y="18">00</text><text x="146" y="84">06</text>
              <text x="80" y="155">12</text><text x="14" y="84">18</text>
              <text x="80" y="78">{clockTime(m().minute)}</text><text class="background-clock-unit" x="80" y="94">24 hours</text>
            </svg>
            <div class="background-clock-copy">
              <div class="background-clock-heading"><strong>{m().date}</strong><span>{m().timeZone}</span><span>{bands.length} windows</span><button class="btn-link" onClick={props.refresh}>Refresh windows</button></div>
              <p>Configured hours, not run status. Local clock hours may repeat or skip at daylight saving.</p>
              <ul aria-label="Configured background windows">
                <For each={bands}>{band => <li>
                  <button type="button" class="background-clock-link" onClick={() => openAutomationsView('automations')}
                    aria-label={`${band.label}: ${ranges(band)}. Open Automations`}>
                    <span>{band.label}</span><span>{ranges(band)}</span>
                  </button>
                </li>}</For>
              </ul>
              <Show when={m().bands.length > 6}><p>First six windows on the dial; all windows listed.</p></Show>
              <Show when={!m().tonightKnown}><p>Tonight’s window is unknown.</p></Show>
            </div>
          </div>
        }</Show>
      </Show>
    </Show>
    </details>
  </section>;
}
