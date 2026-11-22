import { For, Show, type JSX, createEffect, createMemo, onCleanup, onMount } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { missionTracks, trackCost, trackPhase, type MissionTrack } from '../missionTracks';
import { money, type Feed, type MissionCard } from '../missionControl';

/** Sequence tracks, not a fabricated wall-clock/Git timeline. Stays in the lazy chunk. */
export function MissionTracks(props: {
  empty?: JSX.Element;
  live: MissionCard[]; finished: MissionCard[]; feed: Feed; selected: string | null;
  word: (card: MissionCard) => string; onSelect: (id: string) => void;
}) {
  const [board, setBoard] = createStore<{ tracks: MissionTrack[] }>({ tracks: [] });
  createEffect(() => setBoard('tracks', reconcile(missionTracks(props.live, props.finished), { key: 'id' })));
  const tracks = () => board.tracks;
  const costs = createMemo(() => trackCost([...props.live, ...props.finished]));
  return <section class="mc-tracks" aria-label="Running tracks">
    <Show when={tracks().length}><header class="mc-track-heading">
      <div><h3>Work in motion</h3><p>Live turns and today’s finishes, grouped by workspace. Segments show sequence, not duration.</p></div>
      <div class="mc-track-cost" role="group" aria-label="Cost coverage for these turns">
        <strong>{money(costs().knownUsd) ?? 'Unknown'}</strong><span>recorded for these turns</span>
        <svg viewBox="0 0 120 8" width="120" height="8" aria-hidden="true">
          <rect class="mc-cost-rail" width="120" height="8" rx="4" />
          <rect class="mc-cost-known" width={costs().total ? 120 * costs().known / costs().total : 0} height="8" rx="4" />
        </svg>
        <small>{costs().known} of {costs().total} with cost · {costs().unknown} unknown · {costs().pending} pending</small>
      </div>
    </header></Show>
    <Show when={tracks().length} fallback={props.empty ?? <p class="mc-quiet">No turns to show yet.</p>}>
      <For each={tracks()}>{track => <section class="mc-track-lane" aria-label={track.title}>
        <header><h4>{track.title}</h4><span>{track.live} live · {track.cards.length} turns</span></header>
        <div class="mc-track-segments" role="group" aria-label={`${track.title} turns`}>
          <For each={track.cards}>{c => <TrackSegment card={c} phase={props.word(c) === 'Waiting on you' ? 'Waiting on you' : trackPhase(c, props.feed)} selected={props.selected === c.id} onSelect={() => props.onSelect(c.id)} />}</For>
        </div>
      </section>}</For>
    </Show>
    <Show when={tracks().length}><p class="mc-track-footnote">Finished means the turn ended. Gate success and merged changes need their own evidence. Costs can arrive at turn end; this view is not an overnight total.</p></Show>
  </section>;
}

function TrackSegment(props: { card: MissionCard; phase: string; selected: boolean; onSelect: () => void }) {
  let root!: HTMLButtonElement;
  onMount(() => {
    let visible = false;
    const sync = () => { root.dataset.visible = String(visible && !document.hidden); };
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
      visible = entries.some(e => e.isIntersecting && e.intersectionRatio > 0); sync();
    });
    observer?.observe(root);
    document.addEventListener('visibilitychange', sync);
    onCleanup(() => { observer?.disconnect(); document.removeEventListener('visibilitychange', sync); });
  });
  const active = () => props.card.status === 'running' && !!props.card.tool && props.phase !== 'Waiting on you';
  return <button ref={root} type="button" class="mc-track-segment" data-mc-id={props.card.id} data-visible="false"
    data-state={props.card.status} data-active={active()} aria-current={props.selected ? 'true' : undefined}
    aria-label={`${props.card.title}, ${props.phase}, ${props.card.systemTitle}`} title={`${props.card.title} · ${props.phase}`}
    onClick={props.onSelect} onFocus={props.onSelect}>
    <span class="mc-track-phase">{props.phase}<Show when={active()}><span class="mc-track-tool-word"> · tool active</span></Show></span>
    <svg viewBox="0 0 180 28" width="180" height="28" fill="none" aria-hidden="true">
      <path class="mc-track-wire" d="M0 14H180" />
      <rect class="mc-track-block" x="4" y="7" width="172" height="14" rx="7" />
      <Show when={props.card.status === 'done'}><path class="mc-track-check" d="m83 14 4 4 9-9" /></Show>
      <Show when={props.card.status === 'failed' || props.card.status === 'interrupted'}><path class="mc-track-check" d="m86 10 8 8m0-8-8 8" /></Show>
      <Show when={active()}><circle class="mc-track-spark" cx="12" cy="14" r="3" /></Show>
    </svg>
    <strong>{props.card.title}</strong>
    <small>{props.card.agent ?? (props.card.source === 'chat' ? 'Agent' : 'Automation')}</small>
  </button>;
}
