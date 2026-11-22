// Settings, Sources, Context, Personal: "Set it up for me" (CX18), a card
// only. It shows the repo name (editable, checked), Private as fixed, the one
// `gh repo create` line with Copy, the address to connect and what Connect
// then does. Hollow creates nothing here: no repo, push, clone, network call
// or disk write; opening and closing it only changes signals. When the login
// is known it fills the row's Connect field (`prefillRemote` never replaces
// what the person typed). Words in `contextLayers.ts`; lazy with the panel.
import { For, Show, createEffect, createSignal, createUniqueId, onMount } from 'solid-js';
import { Button, ErrorText } from '../components/ui';
import { errMsg } from '../utils';
import {
  SETUP_CONNECT, SETUP_CONNECT_FILLED, SETUP_COPIED, SETUP_NOTE, SETUP_OWNER_HINT, SETUP_REPO_DEFAULT, SETUP_RUN, SETUP_THEN, SETUP_VISIBILITY,
  prefillRemote, setupCard,
} from '../contextLayers';

export function ContextSetupCard(props: {
  /** The GitHub login when Hollow knows it, else null (the placeholder shows). */
  owner: string | null;
  /** The row's Connect field. */
  remote: string;
  setRemote: (url: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = createSignal(SETUP_REPO_DEFAULT);
  const [copied, setCopied] = createSignal('');
  const [copyError, setCopyError] = createSignal('');
  const card = () => setupCard(name(), props.owner);
  // Unique ids: the panel can render in Settings and in the first-run sheet at once.
  const id = createUniqueId();
  let input: HTMLInputElement | undefined;
  onMount(() => input?.focus());

  // Fill the Connect field while it is empty or still holds the card's own address.
  let lastPrefill: string | null = null;
  createEffect(() => {
    const next = card().prefill;
    const value = prefillRemote(props.remote, lastPrefill, next);
    lastPrefill = next;
    if (value !== props.remote) props.setRemote(value);
  });

  const copy = () => {
    const command = card().command;
    if (!command) return;
    setCopied('');
    setCopyError('');
    // Written in the click's own tick: WebKit drops the user gesture across an await.
    navigator.clipboard.writeText(command).then(() => setCopied(SETUP_COPIED), e => setCopyError(`Couldn’t copy: ${errMsg(e)}`));
  };

  return (
    <section class="context-setup-card" data-setup-card aria-labelledby={`${id}-title`}
      onKeyDown={e => { if (e.key === 'Escape') { e.stopPropagation(); props.onClose(); } }}>
      <div class="settings-row-k" id={`${id}-title`}>Set up a private repo</div>
      <p class="task-tracker-value" data-setup-note>{SETUP_NOTE}</p>
      <div class="task-tracker-fields">
        <div class="task-tracker-field">
          <label class="task-tracker-label" for={`${id}-name`}>Repo name</label>
          <input id={`${id}-name`} ref={input} class="input" autocomplete="off" spellcheck={false}
            value={name()} onInput={e => { setName(e.currentTarget.value); setCopied(''); }}
            aria-invalid={!!card().problem} aria-describedby={card().problem ? `${id}-problem` : undefined} />
        </div>
        <Show when={card().problem}>{why => <p class="settings-row-warn" id={`${id}-problem`}>{why()}</p>}</Show>
        <div class="task-tracker-field">
          <span class="task-tracker-label">Visibility</span>
          <span class="task-tracker-value" data-setup-visibility>{SETUP_VISIBILITY}</span>
        </div>
        <ol class="context-setup-steps task-tracker-value">
          <li>
            <p>{SETUP_RUN}</p>
            <div class="task-tracker-field">
              <code class="context-setup-code" data-setup-command>{card().command ?? 'Fix the name first.'}</code>
              <Button size="sm" variant="secondary" disabled={!card().command} onClick={copy}>Copy command</Button>
            </div>
            <Show when={copied()}><p class="task-tracker-value" role="status">{copied()}</p></Show>
            <Show when={copyError()}><ErrorText as="p" class="settings-row-warn" text={copyError()} /></Show>
          </li>
          <li>
            <p>{card().prefill ? SETUP_CONNECT_FILLED : SETUP_CONNECT}</p>
            <code class="context-setup-code" data-setup-url>{card().url ?? 'Fix the name first.'}</code>
            <Show when={card().url && !card().prefill}><p>{SETUP_OWNER_HINT}</p></Show>
          </li>
          <li>
            <For each={SETUP_THEN}>{line => <p>{line}</p>}</For>
          </li>
        </ol>
        <div class="task-tracker-field">
          <Button size="sm" variant="ghost" onClick={() => props.onClose()}>Close</Button>
        </div>
      </div>
    </section>
  );
}
