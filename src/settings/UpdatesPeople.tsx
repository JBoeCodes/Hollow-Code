import { For, Show, createEffect, createSignal, createUniqueId, on, onCleanup } from 'solid-js';
import { api } from '../api';
import { Spinner } from '../components/Spinner';
import { ErrorText } from '../components/ui';
import type { UpdateChannel, UpdatePerson, UpdateSettings } from '../generated/ipc';
import { showConfirm } from '../store';
import { updateState } from '../store/updates';
import { errMsg } from '../utils';
import '../styles/updates-cards.css';

export function UpdatesPeople(props: { settings: UpdateSettings | undefined; onSettings: (next: UpdateSettings) => void }) {
  const id = createUniqueId();
  const [people, setPeople] = createSignal<UpdatePerson[] | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [added, setAdded] = createSignal<{ name: string; link: string } | null>(null);
  const [copied, setCopied] = createSignal(false);
  let alive = true;
  let nameInput: HTMLInputElement | undefined;
  let newMain: HTMLInputElement | undefined;
  let newNightly: HTMLInputElement | undefined;
  onCleanup(() => { alive = false; setAdded(null); });
  const publisher = () => props.settings?.publisher ?? false;
  async function act(fn: () => Promise<unknown>) {
    if (busy()) return;
    setBusy(true); setError('');
    try { await fn(); }
    catch (e) { if (alive) setError(errMsg(e)); }
    finally { if (alive) setBusy(false); }
  }
  const load = () => act(async () => { const list = await api.updatePeopleList(); if (alive) setPeople(list.people); });
  createEffect(on(publisher, enabled => { if (enabled && !people()) void load(); if (!enabled) { setAdded(null); setCopied(false); } }));
  function channelText(c: UpdateChannel[]) {
    return c.includes('main') && c.includes('nightly') ? 'Main and Nightly' : c.includes('main') ? 'Main' : c.includes('nightly') ? 'Nightly' : 'No channel';
  }
  function replace(person: UpdatePerson) { setPeople(list => (list ?? []).map(p => p.id === person.id ? person : p)); }
  function channelsChanged(person: UpdatePerson, channel: UpdateChannel, input: HTMLInputElement) {
    const enabled = input.checked;
    input.checked = person.channels.includes(channel);
    const next = (['main', 'nightly'] as UpdateChannel[]).filter(c => c === channel ? enabled : person.channels.includes(c));
    void act(async () => {
      const p = await api.updatePeopleSetChannels(person.id, next);
      if (alive) replace(p);
    });
  }
  async function add() {
    if (busy()) return;
    const name = nameInput?.value ?? '';
    const channels = [newMain?.checked && 'main', newNightly?.checked && 'nightly'].filter(Boolean) as UpdateChannel[];
    if (!name.trim()) { setError('Type a name first.'); return; }
    if (!channels.length) { setError('Choose at least one channel.'); return; }
    if (!await showConfirm({ title: `Add ${name.trim()}?`, body: `Hollow makes an install link for ${channelText(channels)} updates and shows it once.`, confirmLabel: 'Add person' }) || !alive) return;
    setAdded(null); setCopied(false);
    await act(async () => {
      const result = await api.updatePeopleAdd(name.trim(), channels);
      if (!alive) return;
      setPeople(list => [...(list ?? []), result.person]);
      setAdded({ name: result.person.name, link: result.installLink });
      if (nameInput) nameInput.value = '';
    });
  }
  async function revoke(person: UpdatePerson) {
    if (busy() || !await showConfirm({ title: `Revoke ${person.name}?`, body: `${person.name}'s Mac stops getting updates at its next check. To let them back, add them again for a new link.`, confirmLabel: 'Revoke', danger: true }) || !alive) return;
    await act(async () => { const p = await api.updatePeopleRevoke(person.id); if (alive) replace(p); });
  }
  function copy() {
    const a = added();
    if (!a) return;
    navigator.clipboard.writeText(a.link).then(() => { if (alive) setCopied(true); }, () => { if (alive) setError('Could not copy the link. Select it and copy it by hand.'); });
  }
  return <section class="updates-card" aria-labelledby={id}>
    <h3 class="updates-card-title" id={id}>People</h3>
    <Show when={updateState()?.personName}>{name => <p class="settings-note">Connected as {name()}.</p>}</Show>
    <div class="settings-row"><div class="settings-row-label">
      <label class="settings-row-k"><input type="checkbox" checked={publisher()} disabled={busy() || !props.settings} onChange={event => {
        const enabled = event.currentTarget.checked;
        event.currentTarget.checked = publisher();
        void act(async () => {
          const next = await api.updateSetPublisher(enabled);
          if (alive) props.onSettings(next);
        });
      }} /> This Mac publishes updates</label>
      <div class="settings-row-v">Shows Publish to Main and the People list. Builds run on this Mac's runner.</div>
    </div></div>
    <Show when={busy()}><p class="updates-card-busy" role="status"><Spinner /> Asking the release builder. This can take up to 90 seconds.</p></Show>
    <Show when={publisher()}>
      <Show when={people()}>{list => <Show when={list().length} fallback={<p class="settings-note">No one else gets updates yet.</p>}>
        <ul class="updates-card-list"><For each={list()}>{person => <li>
          <span>{person.name}</span>{' '}<span class="settings-note">{person.revoked ? 'Revoked' : channelText(person.channels)}</span>
          <Show when={!person.revoked}><div class="updates-card-actions">
            <label><input type="checkbox" checked={person.channels.includes('main')} disabled={busy()} onChange={e => channelsChanged(person, 'main', e.currentTarget)} /> Main</label>
            <label><input type="checkbox" checked={person.channels.includes('nightly')} disabled={busy()} onChange={e => channelsChanged(person, 'nightly', e.currentTarget)} /> Nightly</label>
            <button type="button" class="btn" disabled={busy()} onClick={() => void revoke(person)}>Revoke</button>
          </div></Show>
        </li>}</For></ul>
      </Show>}</Show>
      <div class="settings-row"><label class="settings-row-label"><span class="settings-row-k">Name</span><input ref={nameInput} type="text" maxLength={80} autocomplete="off" disabled={busy()} /></label></div>
      <div class="updates-card-actions">
        <label><input ref={newMain} type="checkbox" checked disabled={busy()} /> Main</label>
        <label><input ref={newNightly} type="checkbox" disabled={busy()} /> Nightly</label>
        <button type="button" class="btn" disabled={busy()} onClick={() => void add()}>Add person</button>
      </div>
      <Show when={added()}>{a => <div class="updates-link" role="status">
        <p>Send this link to {a().name}. Hollow shows it only now; if it is lost, revoke and add them again.</p>
        <p><code>{a().link}</code></p>
        <div class="updates-card-actions">
          <button type="button" class="btn" disabled={busy()} onClick={copy}>{copied() ? 'Copied' : 'Copy link'}</button>
          <button type="button" class="btn" disabled={busy()} onClick={() => { setAdded(null); setCopied(false); }}>Done</button>
        </div>
      </div>}</Show>
      <button type="button" class="btn" disabled={busy()} onClick={() => void load()}>Refresh</button>
    </Show>
    <Show when={error()}><ErrorText class="settings-error" text={error()} /></Show>
  </section>;
}
