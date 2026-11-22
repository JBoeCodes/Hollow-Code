import { Match, Show, Switch, createUniqueId, onCleanup, onMount } from 'solid-js';
import { MAIN_AGENT_STATE_WORDS, sessionStateLabel, type MainAgentState, type SessionRollup, type SessionState } from '../sessionIndicator';
import '../styles/session-indicator.css';
import '../styles/main-orb.css';
import { ensureCustomIcons, mainAgentIcon } from '../store/customIcons';
import foldMainAgent from '../assets/brand/main-agent-fold-512.png';

/**
 * The one session indicator (asks 10-11 A2, option A "ring"). Each state is
 * its own shape, so colour is a second channel, not the only one:
 * running = an open ring with one turning arc, queued = a dotted ring,
 * waiting on you = a solid disc with a knocked-out "!", failed = a ring with
 * an ×, interrupted (Hollow quit mid-turn, P1-40) = a ring with a pause
 * sign, paused (the person paused it, P1a) = the pause sign alone, no
 * ring. Only running moves (a transform, paused while the window is away,
 * still under either reduced-motion gate). Idle draws nothing. No SVG ids,
 * so any number can share a page. Every surface uses this component, so
 * changing the look is this file and `session-indicator.css`.
 */
export function SessionIndicator(props: { state: SessionState; size?: number; label?: string; decorative?: boolean }) {
  const size = () => props.size ?? 14;
  const label = () => props.label ?? sessionStateLabel(props.state);
  return <Show when={props.state !== 'idle'}>
    <span class="session-indicator" data-state={props.state} role={props.decorative ? undefined : 'img'}
      aria-hidden={props.decorative ? 'true' : undefined} aria-label={props.decorative ? undefined : label()} title={props.decorative ? undefined : label()}>
      <svg viewBox="0 0 16 16" width={size()} height={size()} fill="none" stroke="currentColor" stroke-linecap="round" aria-hidden="true">
        <Switch>
          <Match when={props.state === 'running'}>
            <circle cx="8" cy="8" r="6" stroke-width="1.6" opacity=".28"/>
            <path class="session-indicator-arc" d="M8 2a6 6 0 0 1 6 6" stroke-width="1.8"/>
          </Match>
          <Match when={props.state === 'queued'}>
            <circle cx="8" cy="8" r="6" stroke-width="1.6" stroke-dasharray="1.6 2.6"/>
          </Match>
          <Match when={props.state === 'waiting'}>
            <path stroke="none" fill="currentColor" fill-rule="evenodd" d="M8 1a7 7 0 1 0 0 14A7 7 0 1 0 8 1zM7.1 3.9h1.8v5.3H7.1zM8 10.4a1.15 1.15 0 1 0 0 2.3a1.15 1.15 0 1 0 0-2.3z"/>
          </Match>
          <Match when={props.state === 'failed'}>
            <circle cx="8" cy="8" r="6.2" stroke-width="1.6"/>
            <path d="M5.9 5.9l4.2 4.2M10.1 5.9l-4.2 4.2" stroke-width="1.6"/>
          </Match>
          <Match when={props.state === 'interrupted'}>
            <circle cx="8" cy="8" r="6.2" stroke-width="1.6"/>
            <path d="M6.6 5.6v4.8M9.4 5.6v4.8" stroke-width="1.6"/>
          </Match>
          <Match when={props.state === 'paused'}>
            <path d="M5.8 3.8v8.4M10.2 3.8v8.4" stroke-width="2.2"/>
          </Match>
        </Switch>
      </svg>
    </span>
  </Show>;
}

/** A collapsed workspace's one badge (A1): the most urgent state's mark and,
 *  when more than one conversation is in it, the count. The label names the
 *  chats. Nothing when every conversation is idle. */
export function SessionRollupBadge(props: { rollup: SessionRollup | null; size?: number }) {
  // Not keyed: a new roll-up updates the mark in place, so the arc keeps turning.
  return <Show when={props.rollup}>{r =>
    <span class="session-rollup" data-state={r().state} role="img" aria-label={r().label} title={r().label}>
      <SessionIndicator state={r().state} size={props.size ?? 12} decorative/>
      <Show when={r().count > 1}><span class="session-rollup-count" aria-hidden="true">{r().count}</span></Show>
    </span>}
  </Show>;
}

/** One SVG presence mark for the top bar and the Main agent space.
 * Static shapes carry every state; only visible transform/opacity layers move. */
export function MainAgentIndicator(props: { state: MainAgentState; provider?: string | null; size?: number; waitingCount?: number; decorative?: boolean; label?: string; image?: string | null }) {
  let root!: HTMLSpanElement;
  const clip = `main-icon-${createUniqueId()}`;
  const artwork = () => props.image === undefined ? (mainAgentIcon() ?? foldMainAgent) : props.image;
  const count = () => Math.max(0, Math.floor(props.waitingCount ?? 0));
  const label = () => props.label ?? `COO · ${MAIN_AGENT_STATE_WORDS[props.state]}${props.state === 'waiting' && count() ? ` · ${count()} conversation${count() === 1 ? '' : 's'}` : ''}`;
  onMount(() => {
    ensureCustomIcons();
    let visible = false;
    const sync = () => { root.dataset.visible = String(visible && !document.hidden); };
    // Unsupported observers deliberately keep decoration static.
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
      visible = entries.some(e => e.isIntersecting && e.intersectionRatio > 0); sync();
    });
    observer?.observe(root);
    document.addEventListener('visibilitychange', sync);
    sync();
    onCleanup(() => { observer?.disconnect(); document.removeEventListener('visibilitychange', sync); });
  });
  return <span ref={root} class="main-indicator" data-state={props.state} data-provider={props.provider ?? undefined} data-visible="false"
    role={props.decorative ? undefined : 'img'} aria-hidden={props.decorative ? 'true' : undefined}
    aria-label={props.decorative ? undefined : label()}>
    <svg viewBox="0 0 32 32" width={props.size ?? 24} height={props.size ?? 24} fill="none" stroke="currentColor"
      stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <circle class="main-orb-aura" cx="16" cy="16" r="12" fill="currentColor" stroke="none" opacity=".09"/>
      <Show when={artwork()} fallback={<path class="main-orb-door" d="M11 23V14a5 5 0 0 1 10 0v9zM14 22v-7a2 2 0 0 1 4 0v7" stroke-width="1.5"/>}>{url => <>
        <defs><clipPath id={clip}><circle cx="16" cy="16" r="9.5"/></clipPath></defs>
        <circle cx="16" cy="16" r="11" stroke="var(--canvas)" stroke-width="4"/>
        <image class="main-icon-image" href={url()} x="6" y="6" width="20" height="20" clip-path={`url(#${clip})`} preserveAspectRatio="xMidYMid meet"/>
      </>}</Show>
      <circle class="main-orb-shell" cx="16" cy="16" r="13" stroke-width="1.2" opacity=".4"/>
      <g class="main-orb-orbit">
        <path d="M16 3a13 13 0 0 1 11.3 19.5" stroke-width="2"/>
        <circle cx="27.3" cy="22.5" r="1.4" fill="currentColor" stroke="none"/>
      </g>
      <g class="main-orb-counter">
        <path d="M16 27a11 11 0 0 1-9.5-16.5" stroke-width="1.3" opacity=".65"/>
        <circle cx="6.5" cy="10.5" r="1" fill="currentColor" stroke="none"/>
      </g>
      <Show when={props.state === 'night'}>
        <path class="main-orb-moon" d="M25 2a6 6 0 1 0 5 9 6.5 6.5 0 0 1-5-9Z" fill="var(--surface-card)" stroke-width="1.3"/>
      </Show>
      <Show when={props.state === 'failed'}>
        <path class="main-orb-error" d="m23 23 6 6m0-6-6 6" stroke-width="2.5"/>
      </Show>
    </svg>
    <Show when={props.state === 'waiting'}>
      <span class="main-orb-count" aria-hidden="true">{count() > 99 ? '99+' : count() || '!'}</span>
    </Show>
  </span>;
}
