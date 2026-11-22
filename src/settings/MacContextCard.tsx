import { For, Show, createSignal, onMount } from 'solid-js';
import { Button, ErrorText } from '../components/ui';
import { errMsg } from '../utils';
import {
  macContextAnswer, macContextConnect, macContextOffer,
  type MacContextLayer, type MacContextOffer, type MacContextReport,
} from '../contextLayersApi';
import {
  MAC_LAYER_LABEL, macConnectProblem, macConnectRequest, macContextLead,
  macNeedsUrl, macStepPill, macTickable, macTickedAtFirst, type Pill,
} from '../contextLayers';

export function MacContextCard(props: { onChanged: () => void }) {
  const [offer, setOffer] = createSignal<MacContextOffer | null>(null);
  const [ticked, setTicked] = createSignal<string[]>([]);
  const [urls, setUrls] = createSignal({ personal: '', org: '' });
  const [slug, setSlug] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [report, setReport] = createSignal<MacContextReport | null>(null);

  async function reload() {
    let next = await macContextOffer(false);
    if (!next) {
      const later = await macContextOffer(true);
      next = later ? { ...later, open: false } : null;
    }
    setOffer(next);
    setTicked(next ? macTickedAtFirst(next) : []);
  }
  // A failed read says so (no silent empty card); the panel's rows still show.
  onMount(() => { void reload().catch(e => { setOffer(null); setError(`Couldn’t read Connect this Mac’s context: ${errMsg(e)}`); }); });

  async function connect() {
    const current = offer();
    if (!current || busy()) return;
    const request = macConnectRequest(current, ticked(), urls(), slug());
    if (macConnectProblem(current, request)) return;
    setBusy(true);
    setError('');
    try {
      setReport(await macContextConnect(request));
      try { await reload(); } finally { props.onChanged(); }
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  async function answer(value: 'not-now' | 'never') {
    const pair = offer()?.pair;
    if (!pair || busy()) return;
    setBusy(true);
    setError('');
    try {
      await macContextAnswer(pair, value);
      setReport(null);
      // Read again: the offer closes and the connect-later fold stays.
      try { await reload(); } catch (e) {
        setOffer(null);
        setError(`Your answer was saved, but the card couldn’t refresh: ${errMsg(e)}`);
      } finally { props.onChanged(); }
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  function Chip(p: { pill: Pill }) {
    return <span class="settings-state-chip" data-state={p.pill.state}>{p.pill.text}</span>;
  }

  function Body(p: { offer: MacContextOffer }) {
    const problem = () => macConnectProblem(p.offer, macConnectRequest(p.offer, ticked(), urls(), slug()));
    const rowPill = (row: MacContextLayer): Pill => row.state === 'connected'
      ? { text: 'Here', state: 'ok' } : row.state === 'held' ? { text: 'Held' } : { text: 'Missing', state: 'warn' };
    return (
      <div class="settings-row-label" aria-busy={busy()}>
        <div class="settings-row-k">Connect this Mac’s context</div>
        <div class="settings-row-v">{macContextLead(p.offer)}</div>
        <div class="task-tracker-fields">
          <For each={p.offer.layers}>{row => (
            <div class="task-tracker-field">
              <label class="task-tracker-label">
                <input type="checkbox" aria-label={MAC_LAYER_LABEL[row.layer]} checked={ticked().includes(row.layer)}
                  disabled={!macTickable(row) || busy()} onChange={e => {
                    const checked = e.currentTarget.checked;
                    setTicked(prev => checked ? [...prev, row.layer] : prev.filter(layer => layer !== row.layer));
                  }} />
                {MAC_LAYER_LABEL[row.layer]}
              </label>
              <span class="task-tracker-value">{row.text}</span>
              <Chip pill={rowPill(row)} />
              <Show when={macNeedsUrl(row) && ticked().includes(row.layer)}>
                <input class="input" aria-label={`${MAC_LAYER_LABEL[row.layer]} repo address`}
                  placeholder={row.url ?? 'git@github.com:owner/repo.git'} autocomplete="off" spellcheck={false}
                  disabled={busy()} value={row.layer === 'personal' ? urls().personal : urls().org}
                  onInput={e => {
                    const key = row.layer === 'personal' ? 'personal' : 'org';
                    setUrls(prev => ({ ...prev, [key]: e.currentTarget.value }));
                  }} />
              </Show>
              <Show when={row.layer === 'home' && ticked().includes('home') && !p.offer.slug}>
                <input class="input" aria-label="Name for this Mac" placeholder={p.offer.suggestedSlug ?? 'my-mac'}
                  autocomplete="off" spellcheck={false} disabled={busy()} value={slug()}
                  onInput={e => setSlug(e.currentTarget.value)} />
              </Show>
            </div>
          )}</For>
          <Show when={problem() && ticked().length > 0}><p class="settings-row-warn">{problem()}</p></Show>
          <div class="task-tracker-field">
            <Button size="sm" variant="secondary" disabled={busy() || problem() !== null} onClick={() => void connect()}>Connect</Button>
            <Show when={p.offer.open && p.offer.pair}>
              <Button size="sm" variant="ghost" disabled={busy()} onClick={() => void answer('not-now')}>Not now</Button>
              <Button size="sm" variant="ghost" disabled={busy()} onClick={() => void answer('never')}>Not for this Mac</Button>
            </Show>
          </div>
        </div>
      </div>
    );
  }

  return <>
    <Show when={offer()}>{current => (
      <Show when={current().open} fallback={
        <Show when={current().layers.some(row => row.state === 'missing')}>
          <details class="settings-fold" data-layer-create="connect-mac">
            <summary>Connect this Mac’s context</summary>
            <Body offer={current()} />
          </details>
        </Show>
      }>
        <section class="settings-row task-tracker-row mac-context-card" data-layer="connect-mac" aria-label="Connect this Mac's context">
          <Body offer={current()} />
        </section>
      </Show>
    )}</Show>
    <Show when={error()}><ErrorText as="p" class="settings-row-warn" text={error()} /></Show>
    <Show when={report()}>{result => <>
      <ul class="mac-context-steps" role="status">
        <For each={result().steps}>{step => <li>
          <Chip pill={macStepPill(step.state)} />{' '}
          {MAC_LAYER_LABEL[step.layer as MacContextLayer['layer']] ?? step.layer}: {step.text}
        </li>}</For>
      </ul>
      <Show when={result().connected}><p class="settings-row-v">This Mac’s context is connected.</p></Show>
    </>}</Show>
  </>;
}
