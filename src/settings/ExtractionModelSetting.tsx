// Settings, Advanced, Extraction model (overnight A11), and
// Settings, Chat, Retitle new chats (P1-22): the model
// the extraction pipeline asks the agent it runs on now. Per agent (an id
// means nothing to another provider); empty is the default (the newest
// Claude Haiku on Claude, the agent's Settings, Models elsewhere). The agent's
// model list only suggests; any id is accepted.
import { For, Show, createResource, createSignal, createUniqueId } from 'solid-js';
import { api } from '../api';
import { agentSettings, flashFooter } from '../store';
import { errMsg } from '../utils';
import { extractionModelNote } from '../extractionModel';
import { ErrorText } from '../components/ui';
import { NewerModelHint } from '../components/NewerModelHint';
import { loadCatalog, withoutChatOnly } from '../modelCatalog';
import { preserveDuringRefetch, savedOrRead } from '../stableResource';
import { settingsRevision } from '../store/settingsRevision';

/** `part`: the Extraction model row (Advanced) or Retitle new chats
 *  (Chat, P1-22); both read and save the same extraction settings. */
export function ExtractionModelSetting(props: { part: 'model' | 'retitle' }) {
  // Re-read when the default agent changes (extraction follows it) and when
  // an agent changed an extraction setting (`settingsRevision`, overnight
  // H6), keeping the shown value while it reads. A save answers through
  // `refetch(saved)` (`savedOrRead`), so a re-read in flight when the person
  // saves can't land after it and show the older value.
  const [read, { refetch }] = createResource(() => `${agentSettings().defaultAgent ?? ''}#${settingsRevision()}`, savedOrRead(() => api.getExtractionSettings()));
  const view = preserveDuringRefetch(read);
  const shown = () => (view.error ? undefined : view());
  const [catalog] = createResource(() => props.part === 'model' && shown()?.agent, a => loadCatalog(a).catch(() => null));
  const [error, setError] = createSignal('');
  const [retitleError, setRetitleError] = createSignal('');
  const retitleId = createUniqueId();

  async function save(model: string) {
    const v = shown();
    if (!v) return;
    setError('');
    try {
      const saved = await api.setExtractionSettings({ agent: v.agent, model: model.trim() });
      refetch(saved);
      flashFooter(saved.chosen ? `Extraction model: ${saved.chosen}` : 'Extraction model: the default', 1800);
    } catch (e) {
      setError(errMsg(e));
    }
  }

  async function setRetitle(box: HTMLInputElement) {
    const on = box.checked;
    setRetitleError('');
    try {
      refetch(await api.setChatRetitle(on));
      flashFooter(on ? 'New chats get a model title after their first answer' : 'New chats keep their first-line title', 2000);
    } catch (e) {
      // Not saved: the box shows what is.
      box.checked = !on;
      setRetitleError(errMsg(e));
    }
  }

  return (
    <>
    <Show when={props.part === 'model'}>
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">Extraction model</div>
        <div class="settings-row-v">
          What reads recordings and chats for commitments, records, decisions and people.{' '}
          <Show when={shown()} fallback={view.error ? `Couldn't read it: ${errMsg(view.error)}` : ''}>{v => <>{extractionModelNote(v())}</>}</Show>
        </div>
        <NewerModelHint value={shown()?.chosen ?? ''} catalog={catalog()} onUse={id => void save(id)} />
        <Show when={error()}>{msg => <ErrorText class="settings-row-warn" as="div" text={msg()}/>}</Show>
      </div>
      <div class="settings-actions">
        <input class="settings-input" type="text" list="extraction-models" aria-label="Extraction model" placeholder="default"
          value={shown()?.chosen ?? ''} disabled={!shown()} style={{ width: '11rem' }}
          onChange={(e) => void save(e.currentTarget.value)} />
        <datalist id="extraction-models">
          <For each={withoutChatOnly(catalog()?.models ?? [])}>{m => <option value={m.id}>{m.label}</option>}</For>
        </datalist>
        <Show when={shown()?.chosen}>
          <button type="button" class="btn ghost small" onClick={() => void save('')}>Use the default</button>
        </Show>
      </div>
    </div>
    </Show>
    <Show when={props.part === 'retitle'}>
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k" id={`${retitleId}-k`}>Retitle new chats</div>
        <div class="settings-row-v" id={`${retitleId}-v`}>
          A short title from each new chat’s first answer. Off: from its first line.
        </div>
        <Show when={retitleError()}>{msg => <ErrorText class="settings-row-warn" as="div" text={msg()}/>}</Show>
      </div>
      <div class="settings-actions">
        <input type="checkbox" aria-labelledby={`${retitleId}-k`} aria-describedby={`${retitleId}-v`} checked={shown()?.retitleChats ?? false} disabled={!shown()}
          onChange={(e) => void setRetitle(e.currentTarget)} />
      </div>
    </div>
    </Show>
    </>
  );
}
