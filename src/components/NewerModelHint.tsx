import { Show } from 'solid-js';
import { newerModel } from '../modelCatalog';
import type { AgentModelCatalog } from '../types';

/** "A newer Sonnet is out: claude-sonnet-5-5 · Use it", under a model
 *  setting that pins an older model of a family. */
export function NewerModelHint(props: {
  value: string; catalog: AgentModelCatalog | null | undefined; onUse: (id: string) => void; disabled?: boolean;
}) {
  const newer = () => newerModel(props.value, props.catalog);
  const family = () => {
    const name = /^claude-([a-z]+)-/.exec(props.value.trim())?.[1] ?? '';
    return name.charAt(0).toUpperCase() + name.slice(1);
  };
  return <Show when={newer()}>{id =>
    <div class="model-newer" role="note">
      <span>A newer {family()} is out: <code>{id()}</code></span>
      <button type="button" class="btn ghost" disabled={props.disabled}
        aria-label={`Use ${id()} instead of ${props.value}`} onClick={() => props.onUse(id())}>Use it</button>
    </div>}
  </Show>;
}
