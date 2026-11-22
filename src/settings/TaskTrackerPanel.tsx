// Settings, Sources, Task tracker (overhaul A14, board S2). The tracker
// Hollow mirrors projects and tasks from: provider, the one allowed team,
// the API key's state (saved, pasted or copied from Keychain; it never
// comes back to the window), last and next sync, the last error, requests
// left this hour, edits waiting to send, Sync now and the pull interval.
// Words and choices live in `trackerSettings.ts`; the calls in
// `trackerApi.ts` (both lazy with Settings, so neither reaches the App chunk).
import { For, Show, createEffect, createResource, createSignal, on, onCleanup, onMount } from 'solid-js';
import { Button, ErrorText } from '../components/ui';
import { Select } from '../components/Select';
import { Switch } from '../components/Switch';
import { MoreFold } from './settingsLayout';
import '../styles/connectors-library.css';
import { flashFooter, showConfirm } from '../store';
import { trackerSyncPass } from '../store/runtime';
import { errMsg } from '../utils';
import {
  clearTrackerKey, copyKeychainTrackerKey, linearLinkSetup, setTrackerKey, setTrackerSettings, trackerStatus, trackerSync,
} from '../trackerApi';
import type { TrackerStatus } from '../generated/ipc';
import {
  ALLOWED_TEAMS, INTERVAL_MINUTES, PERSONAL_TEAMS, canSyncNow, intervalChoice, personalTeamLabel, requestsLeft, syncWhen, teamAllowed, teamLabel,
  trackerHeadline, trackerHealth,
} from '../trackerSettings';

/** The row title the jump list and Service health's Fix button open. */
export const TASK_TRACKER_ROW = 'Task tracker';

export function TaskTrackerPanel(props: { onStatus?: (status: TrackerStatus) => void }) {
  const [status, { mutate, refetch }] = createResource(() => trackerStatus());
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [note, setNote] = createSignal('');
  const [pasted, setPasted] = createSignal('');
  // A clock for "3m ago": the times move while Settings stays open.
  const [now, setNow] = createSignal(Date.now());
  onMount(() => {
    const tick = window.setInterval(() => setNow(Date.now()), 15_000);
    onCleanup(() => window.clearInterval(tick));
  });
  // A background pass (or an agent's Sync now) refreshes the row.
  createEffect(on(trackerSyncPass, pass => { if (pass) void refetch(); }, { defer: true }));
  createEffect(() => { const s = status(); if (s) props.onStatus?.(s); });

  async function act(run: () => Promise<TrackerStatus | void>, done?: string) {
    if (busy()) return;
    setBusy(true);
    setError('');
    setNote('');
    try {
      const next = await run();
      if (next) mutate(next);
      if (done) setNote(done);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  const s = () => status();
  // The Custom link value for Linear's coding-tool setting (a constant from
  // the backend, so the COO's tracker.link_setup returns the same text).
  const [link] = createResource(() => linearLinkSetup().catch(() => null));
  const copyLink = async () => {
    const value = link()?.value;
    if (!value) return;
    try { await navigator.clipboard.writeText(value); flashFooter('Linear link copied'); }
    catch { setError('Copy failed: select the link and copy it by hand.'); }
  };
  const on_ = () => !!s()?.provider;
  const provider = () => s()?.providers.find(p => p.id === s()?.provider);
  const allowed = () => (s()?.provider ? ALLOWED_TEAMS[s()!.provider!] : undefined);
  const providerOptions = () => [
    { value: '', label: 'Off (tasks stay local)' },
    ...(s()?.providers ?? []).map(p => ({ value: p.id, label: p.displayName })),
  ];
  const chooseProvider = (id: string) => void act(async () => {
    if (!id) return setTrackerSettings({ provider: '' });
    const team = ALLOWED_TEAMS[id];
    return setTrackerSettings({ provider: id, ...(team ? { teamId: team.id, teamKey: team.key } : {}) });
  });
  const useTeam = () => void act(() => {
    const team = allowed();
    if (!team) throw new Error('Hollow has no team it may use for this tracker.');
    return setTrackerSettings({ teamId: team.id, teamKey: team.key });
  });
  // "Add Personal": the one personal team Settings offers, saved with its fixed id and key.
  const personal = () => (s()?.provider ? PERSONAL_TEAMS[s()!.provider!] : undefined);
  const addPersonalTeam = () => void act(() => {
    const team = personal();
    if (!team) throw new Error('Hollow has no personal team for this tracker.');
    return setTrackerSettings({ personalTeamId: team.id, personalTeamKey: team.key });
  }, 'Personal added. Its items appear in Tasks after the next sync.');
  const saveKey = () => void act(async () => {
    const key = pasted().trim();
    if (!key) throw new Error('Paste the API key first.');
    const next = await setTrackerKey(key);
    setPasted('');
    return next;
  }, 'Key saved in Hollow’s Keychain bundle.');
  const keychain = () => void act(() => copyKeychainTrackerKey(), 'Key copied from Keychain.');
  const removeKey = () => void (async () => {
    if (!await showConfirm({
      title: `Remove the ${s()?.displayName ?? 'tracker'} key?`,
      body: 'Sync stops until you save a key again. Your task files stay as they are.',
      confirmLabel: 'Remove key',
      danger: true,
    })) return;
    await act(() => clearTrackerKey(), 'Key removed.');
  })();
  const syncNow = () => void act(async () => {
    const outcome = await trackerSync();
    const parts = [`${outcome.pulled} pulled`, `${outcome.pushed} sent`];
    if (outcome.conflicts) parts.push(`${outcome.conflicts} conflict${outcome.conflicts === 1 ? '' : 's'}`);
    if (outcome.droppedOutOfScope) parts.push(`${outcome.droppedOutOfScope} outside the team, ignored`);
    if (outcome.errors.length) throw new Error(`Sync finished with errors: ${outcome.errors.join(' ')}`);
    setNote(`Synced: ${parts.join(', ')}.`);
    return trackerStatus();
  });
  const setInterval_ = (minutes: string) => void act(() => setTrackerSettings({ pollSeconds: Number(minutes) * 60 }));
  // A18: the Friday status (notes in Hollow) and its optional tracker copy.
  const setFriday = (on: boolean) => void act(() => setTrackerSettings({ fridayStatus: on }));
  const setPostStatus = (on: boolean) => void act(() => setTrackerSettings({ postStatusUpdates: on }));

  const keyWords = () => {
    switch (s()?.key) {
      case 'saved': return 'Saved in Hollow’s Keychain bundle.';
      case 'unreadable': return 'Hollow can’t read its saved credentials. Allow Keychain access, or save the key again.';
      default: return 'No key saved.';
    }
  };

  return (
    <div class="settings-row task-tracker-row" data-state={s()?.state ?? 'loading'}>
      <span class="status-conduit settings-conduit" data-state={busy() ? 'busy' : trackerHealth(s())} aria-hidden="true" />
      <div class="settings-row-label">
        <div class="settings-row-k" id="task-tracker-k">{TASK_TRACKER_ROW}</div>
        <div class="settings-row-v" role="status">{trackerHeadline(s())}</div>
        <Show when={status.error}>
          <ErrorText as="p" class="settings-row-warn" text={`Couldn’t read the task tracker settings: ${errMsg(status.error)}`} />
        </Show>
        <Show when={s()}>
          {st => (
            <div class="task-tracker-fields">
              <div class="task-tracker-field">
                <span class="task-tracker-label" id="task-tracker-provider">Tracker</span>
                <Select label="Task tracker" value={st().provider ?? ''} options={providerOptions()} disabled={busy()} onChange={chooseProvider} size="sm" />
              </div>
              <Show when={link()}>
                {setup => (
                  <details class="settings-fold task-tracker-link-fold">
                    <summary>Linear link</summary>
                    <div class="task-tracker-field task-tracker-link">
                      <span class="task-tracker-label" id="task-tracker-link-k">Linear link</span>
                      <code class="task-tracker-value" aria-labelledby="task-tracker-link-k">{setup().value}</code>
                      <Button size="sm" variant="secondary" onClick={() => void copyLink()} title="Copies the link to paste into Linear">Copy</Button>
                    </div>
                    <p class="settings-row-v task-tracker-sub">In Linear, open Settings, then the coding tools, add a Custom link and paste this as its URL. Clicking it on an issue opens Hollow on that issue’s project chat (or the COO) with a draft ready. Nothing is sent until you press Send.</p>
                  </details>
                )}
              </Show>
              <Show when={on_()}>
                <div class="task-tracker-field">
                  <span class="task-tracker-label">Team</span>
                  <span class="task-tracker-value">{teamLabel(st())}</span>
                  <Show when={!teamAllowed(st()) && allowed()}>
                    {team => <Button size="sm" variant="secondary" disabled={busy()} onClick={useTeam}>Use {team().name} ({team().key})</Button>}
                  </Show>
                </div>
                <div class="task-tracker-field" data-team-role="personal">
                  <span class="task-tracker-label" id="task-tracker-personal-k">Your own team</span>
                  <Show when={personalTeamLabel(st())} fallback={
                    <Show when={personal()}>
                      {team => <>
                        <span class="task-tracker-value">Not added.</span>
                        <Button size="sm" variant="secondary" disabled={busy()} onClick={addPersonalTeam} aria-describedby="task-tracker-personal-k">Add {team().name}</Button>
                      </>}
                    </Show>
                  }>
                    {label => <span class="task-tracker-value" data-personal-team={st().personalTeamKey}>{label()}</span>}
                  </Show>
                </div>
                <MoreFold><p class="settings-row-v task-tracker-sub">The shared team ({st().teamKey ?? 'ENG'}) is real project work, and your team sees it. Your own team is for items that only involve you and Hollow. Not shared with the team. Hollow reads and writes these teams only; nothing from another team reaches your vault.</p></MoreFold>
                <div class="task-tracker-field">
                  <span class="task-tracker-label">API key</span>
                  <span class="task-tracker-value" data-key={st().key}>{keyWords()}</span>
                </div>
                <div class="task-tracker-field task-tracker-key">
                  <input class="input" type="password" aria-label={`${st().displayName ?? 'Tracker'} API key`} autocomplete="off" spellcheck={false}
                    placeholder={st().key === 'saved' ? 'Paste a new key to replace it' : 'Paste the API key'}
                    value={pasted()} onInput={e => setPasted(e.currentTarget.value)} disabled={busy()} />
                  <Button size="sm" variant="secondary" disabled={busy() || !pasted().trim()} onClick={saveKey}>Save key</Button>
                  <Show when={provider()?.keychainItem}>
                    <Button size="sm" variant="ghost" disabled={busy()} onClick={keychain} title="Copies the key you saved in Keychain (one macOS prompt the first time)">Use the key saved in Keychain</Button>
                  </Show>
                  <Show when={st().key !== 'missing'}>
                    <Button size="sm" variant="ghost" disabled={busy()} onClick={removeKey}>Remove key</Button>
                  </Show>
                </div>
                <details class="settings-fold task-tracker-sync-fold" open={st().outbox > 0}><summary>Sync details</summary><dl class="task-tracker-sync" aria-label="Sync">
                  <div><dt>Last sync</dt><dd>{syncWhen(st().lastSync, now())}</dd></div>
                  <div><dt>Next sync</dt><dd>{st().state === 'ready' || st().state === 'error' ? syncWhen(st().nextSync, now(), 'when Hollow next checks') : 'after setup'}</dd></div>
                  <div><dt>Requests left</dt><dd>{requestsLeft(st()) ?? 'not reported yet'}</dd></div>
                  <Show when={st().outbox > 0}>
                    <div><dt>Waiting to send</dt><dd>{st().outbox} edit{st().outbox === 1 ? '' : 's'}</dd></div>
                  </Show>
                </dl></details>
                <Show when={st().lastError}>
                  {e => <ErrorText as="p" class="settings-row-warn task-tracker-error" text={`Last error: ${e()}`} />}
                </Show>
                <div class="task-tracker-field">
                  <span class="task-tracker-label">Check every</span>
                  <Select label="Sync interval" size="sm" disabled={busy()} value={String(intervalChoice(st().pollSeconds))}
                    options={INTERVAL_MINUTES.map(m => ({ value: String(m), label: m === 1 ? '1 minute' : `${m} minutes` }))}
                    onChange={setInterval_} />
                  <Button size="sm" variant="primary" disabled={busy() || !canSyncNow(st())} onClick={syncNow} aria-describedby="task-tracker-k">
                    {busy() ? 'Working…' : 'Sync now'}
                  </Button>
                </div>
                <details class="settings-fold task-tracker-friday-fold"><summary>Friday status</summary>
                  <div class="task-tracker-field">
                    <span class="task-tracker-label">Friday status</span>
                    <Switch checked={st().fridayStatus} disabled={busy()} label="Write each tracked project’s status at 3 PM on Fridays" onChange={setFriday} />
                    <span class="task-tracker-value">Write each tracked project’s status at 3 PM on Fridays</span>
                  </div>
                  <div class="task-tracker-field">
                    <span class="task-tracker-label">Status copy</span>
                    <Switch checked={st().postStatusUpdates} disabled={busy()} label={`Also post it to ${st().displayName ?? 'the tracker'} as a project update`} onChange={setPostStatus} />
                    <span class="task-tracker-value">Also post it to {st().displayName ?? 'the tracker'} as a project update</span>
                  </div>
                  <MoreFold><p class="settings-row-v task-tracker-sub">The status goes into Hollow first: each project’s outputs/status, the COO’s digest and a notification. The copy is optional, team {st().teamKey ?? 'ENG'} only. A project’s Write status now writes one at any time.</p></MoreFold>
                </details>
              </Show>
              <Show when={st().restartNeeded}>
                <p class="settings-row-warn">Saved. The new tracker settings apply after Hollow restarts.</p>
              </Show>
              <Show when={st().problems.length}>
                <ul class="settings-row-warn task-tracker-problems" aria-label="Ignored settings">
                  <For each={st().problems}>{p => <li>{p}</li>}</For>
                </ul>
              </Show>
            </div>
          )}
        </Show>
        <div aria-live="polite">
          <Show when={note()}><p class="settings-row-v task-tracker-note">{note()}</p></Show>
          <Show when={error()}>{msg => <ErrorText as="p" class="settings-row-warn" text={msg()} />}</Show>
        </div>
      </div>
    </div>
  );
}
