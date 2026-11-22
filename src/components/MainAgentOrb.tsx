import { Show } from 'solid-js';
import { MainAgentIndicator } from './SessionIndicator';
import { liveChildCount, mainAgentNow, mainAgentOpen, toggleMainAgent } from '../store/mainAgent';
import { keyFor } from '../shortcuts';

/** The Main agent's place at the top of the app (main agent M1): its session
 *  indicator (asks 10-11 A2: the doorway inside the family's ring, turning
 *  while it works, with the "!" disc while something of its own waits on
 *  you). Opens today's Main agent conversation; pressed again, goes back.
 *  Consolidation D (d1acca43) folded it into an Ask button whose click opened
 *  a panel; restored 2026-10-14 on Alex's ask ("the agent button that takes
 *  you to main agent"). Ask ⌄ beside it was removed on 2026-10-21, and the
 *  drawer its ⌥-click opened beside the page on 2026-10-27 (run 15 P3). */
export function MainAgentOrb() {
  const now = () => mainAgentNow();
  const state = () => now().state;
  const label = () => `${mainAgentOpen() ? 'Back from the COO' : 'COO'} · ${now().words.toLowerCase()}${state() === 'working' && now().providerLabel ? ` · running chats: ${now().providerLabel}` : ''}`;
  const title = () => keyFor('main-agent') ? `${label()} (${keyFor('main-agent')})` : label();
  return (
    <button
      type="button"
      class="top-bar-iconbtn main-agent-orb"
      classList={{ open: mainAgentOpen() }}
      data-state={state()}
      onClick={() => void toggleMainAgent()}
      title={title()}
      aria-label={label()}
      aria-pressed={mainAgentOpen()}
    >
      <MainAgentIndicator state={state()} provider={now().provider} waitingCount={now().waitingCount} size={22} decorative />
      <Show when={liveChildCount() > 0 && state() !== 'waiting'}><span class="main-agent-orb-count" aria-hidden="true">{liveChildCount()}</span></Show>
    </button>
  );
}
