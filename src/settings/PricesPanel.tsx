/**
 * Settings, Advanced, Prices (overnight 9 A13, moved 11-08 S5):
 * the price table Hollow uses where an agent can't price a run itself, a
 * Claude model it reports as unpriced and every Codex run. Codex spend is
 * then estimated at the price entered here, kept apart from reported costs.
 * A built-in row can be replaced, and only the person's own rows can be
 * removed. Saving asks once more, since a price changes every later cost of
 * that model (agents reach the same `set_price` through a confirm card).
 */
import { For, Show, createResource, createSignal } from 'solid-js';
import { api } from '../api';
import '../styles/usage-panel.css';
import type { ModelPrice } from '../types';
import { PlainErrorText } from '../components/ui';
import { errMsg } from '../utils';
import { perMillion as usd, priceFromForm, type PriceForm } from '../modelPrices';

const today = () => new Date().toLocaleDateString('en-CA');

export function PricesPanel() {
  const [rows, { mutate }] = createResource(() => api.priceTable());
  const [form, setForm] = createSignal<PriceForm>({ asOf: today() });
  // What waits for the second click: a save, or a removal (both change
  // every later cost of the model).
  const [pending, setPending] = createSignal<{ price: ModelPrice; remove: boolean } | null>(null);
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const field = (key: keyof PriceForm) => (e: Event) => setForm({ ...form(), [key]: (e.currentTarget as HTMLInputElement).value });

  const ask = () => {
    const price = priceFromForm(form());
    if (typeof price === 'string') {
      setError(price);
      return;
    }
    setError('');
    setPending({ price, remove: false });
  };
  const save = async (price: ModelPrice, remove: boolean) => {
    setBusy(true);
    setError('');
    try {
      mutate(await api.setPrice(price, remove));
      setPending(null);
      if (!remove) setForm({ asOf: today() });
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <details class="usage-block usage-prices">
      <summary class="usage-block-title">Prices</summary>
      <p class="usage-explainer">
        Used only where an agent can't price a run itself. Codex reports tokens but no price: with a
        row for its model here, its spend is estimated at the price you entered.
      </p>
      <Show when={rows.error}>
        <div class="settings-banner danger" role="alert"><p>Prices could not be read: <PlainErrorText text={errMsg(rows.error)} /></p></div>
      </Show>
      <Show when={!rows.error && rows()}>
        {r => (
          <table class="usage-table">
            <caption class="sr-only">Prices in dollars per million tokens</caption>
            <thead>
              <tr>
                <th scope="col">Model</th>
                <th scope="col" class="num">Input</th>
                <th scope="col" class="num">Cached</th>
                <th scope="col" class="num">Output</th>
                <th scope="col">As of</th>
                <th scope="col"><span class="sr-only">Actions</span></th>
              </tr>
            </thead>
            <tbody>
              <For each={r()}>
                {p => (
                  <tr>
                    <th scope="row" title={p.source}>{p.model}{p.saved ? '' : ' (built in)'}</th>
                    <td class="num">{usd(p.input)}</td>
                    <td class="num">{usd(p.cacheRead)}</td>
                    <td class="num">{usd(p.output)}</td>
                    <td title={p.source}>{p.asOf}</td>
                    <td>
                      <Show when={p.saved}>
                        <button type="button" class="settings-btn" disabled={busy()} aria-label={`Remove your price for ${p.model}`}
                          onClick={() => setPending({ price: p, remove: true })}>Remove</button>
                      </Show>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        )}
      </Show>
      <fieldset class="usage-price-form">
        <legend>Add or change a price (dollars per million tokens)</legend>
        <input class="settings-input" aria-label="Model id" placeholder="Model id" value={form().model ?? ''} onInput={field('model')} />
        <input class="settings-input" type="number" min="0" max="1000" step="0.01" aria-label="Input price" placeholder="Input" value={form().input ?? ''} onInput={field('input')} />
        <input class="settings-input" type="number" min="0" max="1000" step="0.01" aria-label="Cached input price" placeholder="Cached" value={form().cacheRead ?? ''} onInput={field('cacheRead')} />
        <input class="settings-input" type="number" min="0" max="1000" step="0.01" aria-label="Output price" placeholder="Output" value={form().output ?? ''} onInput={field('output')} />
        <input class="settings-input" aria-label="Source of the price" placeholder="Source (where it comes from)" value={form().source ?? ''} onInput={field('source')} />
        <input class="settings-input" type="date" aria-label="Price as of" value={form().asOf ?? ''} onInput={field('asOf')} />
        <button type="button" class="settings-btn" disabled={busy()} onClick={ask}>Save price…</button>
      </fieldset>
      <Show when={pending()}>
        {p => (
          <div class="settings-banner" role="alertdialog" aria-label="Confirm the price">
            <Show when={p().remove} fallback={
              <p>Save {p().price.model} at {usd(p().price.input)} input, {usd(p().price.cacheRead)} cached, {usd(p().price.output)} output per million tokens? Every later cost of this model uses it.</p>
            }>
              <p>Remove your price for {p().price.model}? Every later cost of this model uses the built-in price, if there is one, or none.</p>
            </Show>
            <button type="button" class="settings-btn" disabled={busy()} onClick={() => void save(p().price, p().remove)}>{p().remove ? 'Remove' : 'Save'}</button>
            <button type="button" class="settings-btn" disabled={busy()} onClick={() => setPending(null)}>Cancel</button>
          </div>
        )}
      </Show>
      <Show when={error()}>
        <div class="settings-banner danger" role="alert"><p><PlainErrorText text={error()} /></p></div>
      </Show>
    </details>
  );
}
