import { render } from 'solid-js/web';
import { For } from 'solid-js';
import { MainAgentIndicator } from '../../../src/components/SessionIndicator';
import { MAIN_AGENT_STATE_WORDS, type MainAgentState } from '../../../src/sessionIndicator';

export function mount() {
  const host = document.createElement('section');
  host.className = 'main-orb-study';
  host.tabIndex = 0;
  host.setAttribute('aria-label', 'COO presence study');
  host.style.cssText = 'position:fixed;inset:0;z-index:99999;overflow:auto;background:var(--canvas);color:var(--ink);padding:32px';
  document.body.append(host);
  const states: MainAgentState[] = ['resting', 'watching', 'working', 'waiting', 'failed', 'night', 'paused'];
  const dispose = render(() => <>
    <h1>COO</h1><p>One presence, wherever you work.</p>
    <div style={{ display: 'grid', 'grid-template-columns': 'repeat(auto-fit, minmax(220px, 1fr))', gap: '24px', 'max-width': '1050px' }}>
      <For each={states}>{state => <section style={{ padding: '24px', border: '1px solid var(--hairline)', 'border-radius': 'var(--r-lg)', background: 'var(--surface-card)' }}>
        <h2 style={{ 'font-size': 'var(--fs-meta)', 'margin-bottom': '24px' }}>{MAIN_AGENT_STATE_WORDS[state]}</h2>
        <div style={{ display: 'flex', gap: '24px', 'align-items': 'center' }}>
          <For each={[22, 26, 40]}>{size => <MainAgentIndicator state={state} size={size} waitingCount={3} />}</For>
        </div>
      </section>}</For>
    </div>
  </>, host);
  return () => { dispose(); host.remove(); };
}
