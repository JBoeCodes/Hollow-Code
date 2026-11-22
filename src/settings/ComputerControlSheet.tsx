// Settings, Connectors, Computer control: the sheet behind the row's "Set up"
// button (Cua CU7). Basic verbs, the sandbox Mac, This Mac's screen, Stop all
// and the Doctor's checks. A lazy chunk: the Settings page only imports it
// when the sheet opens, so none of this is in the App chunk.
import { For, Show, createResource, createSignal } from 'solid-js';
import { api } from '../api';
import { Switch } from '../components/Switch';
import { ErrorText } from '../components/ui';
import { Icon } from '../icons';
import { SettingsSheet } from './SettingsSheet';
import { SettingsSubhead } from './settingsLayout';
import {
  computerControlEnabled, setComputerControlEnabled,
  computerSandboxEnabled, setComputerSandboxEnabled,
  computerThisMacEnabled, setComputerThisMacEnabled,
} from '../store/runtime';
import type { AgentDeniedHost, ComputerCheck, ComputerStatus, DeniedApp } from '../types';

/** Each check's state in words (not colour alone). */
const STATE_WORD: Record<ComputerCheck['state'], string> = { ok: 'OK.', warn: 'Warning.', fail: 'Problem.', unknown: 'Not checked.' };

export default function ComputerControlSheet(props: { onClose: () => void; returnTo?: () => HTMLElement | null | undefined }) {
  const [status, { refetch }] = createResource<ComputerStatus>(() => api.computerStatus());
  const [apps, { mutate: setApps }] = createResource<DeniedApp[]>(() => api.computerDeniedApps());
  const [hostsError, setHostsError] = createSignal('');
  const [hosts, { mutate: setHosts, refetch: refetchHosts }] = createResource<AgentDeniedHost[]>(() => api.hostScopeAgentDenied().then(list => { setHostsError(''); return list; }).catch(e => { setHostsError(String(e)); return []; }));
  const removeHost = async (pattern: string) => {
    try { setHosts(await api.hostScopeRemoveAgentDeny(pattern)); setHostsError(''); } catch (e) { setHostsError(String(e)); }
    refetchHosts();
  };
  const [checks] = createResource<ComputerCheck[]>(() => api.computerDoctor());
  const [stopped, setStopped] = createSignal<number | null>(null);
  const stopAll = async () => {
    const n = await api.computerStopAll().catch(() => 0);
    setStopped(n);
    refetch();
  };
  const copy = (text: string) => { void navigator.clipboard?.writeText(text).catch(() => { /* not allowed here */ }); };
  const row = (icon: 'globe' | 'appWindow' | 'terminal', title: string, hint: string, on: () => boolean, set: (v: boolean) => void, label: string) => (
    <div class="cx-tool">
      <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name={icon} size={16} /></span>
      <div class="cx-row-text"><strong>{title}</strong><small>{hint}</small></div>
      <Switch checked={on()} label={label} onChange={v => { set(v); refetch(); }} />
    </div>
  );
  return (
    <SettingsSheet id="settings-sheet-computer" title="Computer control" returnTo={props.returnTo} onClose={props.onClose}>
      <div class="cx-tools">
        {row('globe', 'Basic verbs', 'Open links and apps. Reading browser tabs or quitting an app asks you first.', computerControlEnabled, setComputerControlEnabled, 'Computer control basic verbs')}
        {row('appWindow', 'Sandbox Mac', 'Agents see and use a separate Mac’s screen. Nothing on your own Mac is touched. Asks once per chat in Ask mode.', computerSandboxEnabled, setComputerSandboxEnabled, 'Sandbox Mac')}
        {row('terminal', 'This Mac’s screen', 'Off unless you turn it on, and not available yet: it needs the screen helper installed first. Each chat and each app will ask first.', computerThisMacEnabled, setComputerThisMacEnabled, 'This Mac’s screen')}
      </div>
      <p class="settings-row-v">A send, post, payment or delete, and typing into a password field, always ask, even in Bypass.</p>
      <SettingsSubhead title="Apps agents can’t use on this Mac" hint="Blocked in every mode, even Bypass, until you allow them." />
      <ul class="settings-doctor" aria-label="Apps agents can’t use on this Mac">
        <For each={apps() ?? []}>
          {a => (
            <li data-allowed={a.allowed ? 'true' : 'false'}>
              <strong>{a.label}</strong>{' '}<span>{a.allowed ? 'Allowed.' : 'Blocked.'}</span>{' '}
              <button class="btn small ghost" aria-label={`${a.allowed ? 'Block' : 'Allow'} ${a.label} on this Mac`}
                onClick={async () => setApps(await api.computerSetAppAllowed(a.label, !a.allowed))}>
                {a.allowed ? 'Block' : 'Allow'}
              </button>
            </li>
          )}
        </For>
      </ul>
      <SettingsSubhead title="Sites agents blocked for themselves" hint="An agent can add a site here to stop itself reading it. Only you can remove one." />
      <Show when={hostsError()}>
        <ErrorText class="settings-row-v" text={hostsError()} />
        <p class="settings-row-v">
          While this list can’t be read, every site is blocked for agents.{' '}
          <button class="btn small" aria-label="Reset the list of sites agents blocked for themselves" onClick={() => removeHost('')}>Reset the list</button>
        </p>
      </Show>
      <Show when={(hosts() ?? []).length > 0} fallback={<p class="settings-row-v">None. Agents have not blocked any site.</p>}>
        <ul class="settings-doctor" aria-label="Sites agents blocked for themselves">
          <For each={hosts() ?? []}>
            {h => (
              <li>
                <strong>{h.pattern}</strong>{' '}<small>Added {h.addedAt.slice(0, 10)} by {h.by}.</small>{' '}
                <button class="btn small ghost" aria-label={`Remove ${h.pattern} from the sites agents blocked`}
                  onClick={() => removeHost(h.pattern)}>
                  Remove
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <div class="cx-tool">
        <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name="x" size={16} /></span>
        <div class="cx-row-text">
          <strong>Stop all computer control</strong>
          <small>{status() ? `${status()!.liveSessions} running now. ` : ''}Ends every desktop session and forgets what you allowed.</small>
          <Show when={stopped() !== null}><small role="status">{stopped() === 0 ? 'Nothing was running.' : `Ended ${stopped()} session${stopped() === 1 ? '' : 's'}.`}</small></Show>
        </div>
        <button class="btn small" aria-label="Stop all computer control" onClick={stopAll}>Stop all</button>
      </div>
      <SettingsSubhead title="Doctor" hint="What Computer control needs, and the fix for what is missing." />
      <Show when={checks()} fallback={<p class="settings-row-v">Checking…</p>}>
        <ul class="settings-doctor" aria-label="Computer control checks">
          <For each={checks()}>
            {r => (
              <li data-state={r.state}>
                <em>{STATE_WORD[r.state]}</em>{' '}<strong>{r.label}</strong>{' '}<span>{r.line}</span>
                <Show when={r.fix}>
                  {' '}<small>{r.fix}</small>{' '}
                  <button class="btn small ghost" aria-label={`Copy the fix for ${r.label}`} onClick={() => copy(r.fix!)}>Copy fix</button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </SettingsSheet>
  );
}
