import { For, Show, createEffect, createResource, createSignal } from 'solid-js';
import { Select } from './Select';
import { checkedLabel, loadCatalog, withoutChatOnly } from '../modelCatalog';
import { NewerModelHint } from './NewerModelHint';
import '../styles/model-picker.css';

const CUSTOM = '__custom__';

/** A model menu for one agent: its known models, the default, or any custom
 *  ID. Suggestions never validate or rewrite a provider-owned model ID.
 *  Self-hosted models are left out: a chat's own model menu offers them
 *  (self-hosted models Q5), and the defaults and automation overrides this
 *  picker sets can't run them. */
export function ModelPicker(props: {
  agent: string; value: string; onChange: (value: string) => void;
  label: string; id?: string; disabled?: boolean; defaultLabel?: string; compact?: boolean;
}) {
  const [refreshing, setRefreshing] = createSignal(false);
  const [catalog, { refetch }] = createResource(() => props.agent || false, agent => loadCatalog(agent, refreshing()));
  const [custom, setCustom] = createSignal(false);
  createEffect(() => { props.agent; setCustom(false); });
  const models = () => withoutChatOnly(catalog()?.models ?? []);
  const known = () => models().some(model => model.id === props.value);
  // A saved ID the catalog doesn't list stays visible and editable.
  const customVisible = () => custom() || (!!props.value && !catalog.loading && !known());
  async function refresh() {
    setRefreshing(true);
    try { await refetch(); } finally { setRefreshing(false); }
  }
  const meta = () => {
    const c = catalog();
    if (!c) return '';
    if (c.warning) return c.warning;
    const when = checkedLabel(c.checkedAt);
    return when ? `${c.source} · ${when}` : c.source;
  };
  return <div class="model-picker" classList={{compact:!!props.compact}}>
    <Select id={props.id} size={props.compact ? 'sm' : undefined} class={props.compact ? 'chat-model-picker' : undefined} label={props.label} disabled={props.disabled}
      value={customVisible() ? CUSTOM : props.value}
      onChange={value => { setCustom(value === CUSTOM); if (value !== CUSTOM) props.onChange(value); }}
      options={[
        { value: '', label: props.defaultLabel ?? 'Runtime default' },
        ...(catalog.loading && props.value ? [{ value: props.value, label: props.value }] : []),
        ...models().map(model => ({ value: model.id, label: model.label, detail: model.label === model.id ? undefined : model.id, keywords: model.id })),
        { value: CUSTOM, label: 'Custom model ID…' },
      ]} />
    <Show when={customVisible()}>
      <input class={props.compact ? 'chat-model-picker' : undefined} aria-label={`${props.label} custom ID`} type="text" value={props.value}
        disabled={props.disabled} placeholder="Exact model ID" spellcheck={false}
        onInput={event => props.onChange(event.currentTarget.value.trim())} />
    </Show>
    <Show when={!props.compact}>
      <NewerModelHint value={props.value} catalog={catalog()} disabled={props.disabled} onUse={props.onChange} />
    </Show>
    <div class="model-picker-meta">
      <span role="status">{catalog.loading ? 'Loading models…' : meta()}</span>
      <button type="button" class="btn ghost" aria-label={`Refresh ${props.label} choices`} disabled={props.disabled || catalog.loading || !props.agent} onClick={refresh}>Refresh</button>
    </div>
  </div>;
}
