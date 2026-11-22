import { createEffect, createMemo, createSignal, For, Index, onMount, Show } from 'solid-js';
import { api } from '../api';
import { flashFooter, openInBrowser, showConfirm, trackTask } from '../store';
import { Icon } from '../icons';
import { Switch } from '../components/Switch';
import CalendarSource from './CalendarSource';
import { LeadText } from './settingsLayout';
import { SettingsSheet, openerRef } from './SettingsSheet';
import { hueFor } from '../utils';
import {
  AGENT_NAMES, AUTO_APPROVE_HINT, AUTO_APPROVE_LABEL, CLAUDE_AI_CONNECTORS_URL, SOURCE_NAMES, alwaysLoginWarning, alwaysSpreadHint, describeLogin, hollowEquivalent, hollowSignInFor, iconFor, inputFrom, matchesQuery, parseArgs,
  secretsFrom, shareableAgents, toggleAgent, GITHUB_PRESET, SERVICE_SETUP_DEFAULTS, githubConnection,
  type ConnectorInput, type ConnectorStatus, type ConnectorTransport, type ConnectorView,
  type ConnectorsLibraryView, type DiscoveredConnector, type SharedAgent,
  approvedMail, SUPERHUMAN_URL, clientSecretInput, savedSecretFate,
} from '../connectorsLibrary';
import { connectorCapability, evidenceLine, worksOn, type ConnectorCapability, type RepairKind } from '../connectorCapability';
import { connectorChecks, m365Check, recordConnectorChecks, recordM365Check } from '../store/connectorChecks';
import '../styles/connectors-library.css';
import { Skeleton, ErrorText } from '../components/ui';
import { createIconSrc } from '../remoteImageLoader';

interface SecretRow { key: string; value: string; saved: boolean }
interface Draft {
  preset?: 'github';
  id?: string; name: string; description: string; transport: ConnectorTransport;
  command: string; args: string; url: string; secrets: SecretRow[];
  /** Hollow signs in to the remote server itself (five asks B2). */
  oauth: boolean; oauthClientId: string;
  /** A new client secret ('' keeps the saved one), whether one is saved, and
   *  whether to remove it (overnight 10-11 Y6). */
  oauthClientSecret: string; secretSaved: boolean; clearSecret: boolean;
  /** The client id and URL the saved secret was saved with. */
  savedClientId: string; savedUrl: string;
}
const EMPTY: Draft = { name: '', description: '', transport: 'http', command: '', args: '', url: '', secrets: [], oauth: false, oauthClientId: '', oauthClientSecret: '', secretSaved: false, clearSecret: false, savedClientId: '', savedUrl: '' };

function errText(e: unknown) { return e instanceof Error ? e.message : String(e); }

/** The connector's app icon, or a lettered tile when there is none (or it
 *  fails to load). Favicons are remote, and the main window's CSP allows no
 *  remote images, so they come through the backend (`createIconSrc`). */
export function ConnectorIcon(props: { name: string; src: string | null; size?: 'sm' | 'md' }) {
  const [failed, setFailed] = createSignal(false);
  const src = createIconSrc(() => props.src);
  return <span class={`cx-icon cx-icon-${props.size ?? 'md'}`} aria-hidden="true" style={{ '--cx-hue': String(hueFor(props.name)) }}>
    <Show when={src() && !failed()} fallback={<span class="cx-icon-letter">{props.name.trim().charAt(0).toUpperCase() || '?'}</span>}>
      <img src={src()!} alt="" loading="lazy" onError={() => setFailed(true)} />
    </Show>
  </span>;
}

/** Settings, Connectors: the connectors your agents can use (Rust
 *  `connectors.rs`). "Your connectors" lists what's added, each with its
 *  capability (next-level P16: one vocabulary, what was tested, as whom,
 *  when, where it applies, and one repair) and a switch; "Add connectors" is
 *  a searchable gallery of what Hollow found on claude.ai and in Claude Code /
 *  Codex, plus a custom connector form. Plain signals, so a save never
 *  suspends the page. Check results live in `store/connectorChecks`, which
 *  the composer's next-turn list reads too. */
export default function ConnectorsLibrary(props: {
  onState?: (connectors: ConnectorView[], statuses: Record<string, ConnectorStatus>) => void;
} = {}) {
  const [view, setView] = createSignal<ConnectorsLibraryView | null>(null);
  const [error, setError] = createSignal('');
  const statuses = connectorChecks;
  const [checking, setChecking] = createSignal(false);
  const [m365Checking, setM365Checking] = createSignal(false);
  const [open, setOpen] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal<Draft | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [query, setQuery] = createSignal('');
  const [adding, setAdding] = createSignal<string | null>(null);
  // Settings PR 5: the catalog and the connector form are sheets. One opener
  // serves both: the form opened from the catalog returns to Add a connector.
  const [catalog, setCatalog] = createSignal(false);
  const opener = openerRef('[data-sheet-opener="connector-add"]');
  let root: HTMLElement | undefined;
  function focusService(selector: string) {
    const target = root?.querySelector<HTMLElement>(selector) ?? document.querySelector<HTMLElement>(selector);
    target?.scrollIntoView({ block: 'nearest' });
    target?.focus({ preventScroll: true });
  }

  async function setupGitHub() {
    if (!view() || error() || saving()) return;
    const existing = githubConnection(connectors());
    if (existing) {
      setOpen(existing.id);
      const row = document.getElementById(`connector-${existing.id}`);
      row?.scrollIntoView({ block: 'nearest' });
      row?.querySelector<HTMLElement>('.cx-row-toggle')?.focus({ preventScroll: true });
      return;
    }
    const previous = draft();
    if (previous && !await showConfirm({ title: 'Replace this connector draft?', body: 'The connector form has unsaved changes. Replace it with GitHub setup?', confirmLabel: 'Replace draft' })) return;
    // An answer belongs only to the exact form it was asked about.
    if (draft() !== previous || saving() || error()) return;
    // A library refresh may have found an existing connection while the
    // replacement question was open. Keep the draft and use that connection.
    if (githubConnection(connectors())) { void setupGitHub(); return; }
    setDraft({ ...EMPTY, ...GITHUB_PRESET, preset: 'github', secrets: [{ key: 'Authorization', value: '', saved: false }] });
    focusService('.cx-form input[aria-label="Connector name"]');
  }

  const connectors = () => view()?.connectors ?? [];
  const capOf = (c: ConnectorView): ConnectorCapability => connectorCapability(c, {
    status: statuses()[c.id], checking: c.transport === 'builtin' ? m365Checking() : checking(),
    login: view()?.claudeaiLogin, m365: m365Check(),
  });
  // Report what the library already loaded to its host (the service summary,
  // audit H04) so nothing is fetched twice. A connected answer that no longer
  // counts (another login, too old) is left out, so it can't read as working.
  createEffect(() => props.onState?.(connectors(), Object.fromEntries(Object.entries(statuses()).filter(([id, s]) => {
    const c = connectors().find(x => x.id === id);
    return !!c && (s.state !== 'connected' || capOf(c).state === 'available');
  }))));
  const hasClaudeAi = () => connectors().some(c => c.transport === 'claudeAi' && c.enabled);
  const m365Row = () => connectors().find(c => c.transport === 'builtin' && c.builtin?.kind === 'm365' && c.enabled);

  async function load(refresh = false) {
    try { setView(await api.listConnectors(refresh)); setError(''); } catch (e) { setError(errText(e)); }
  }
  async function checkStatus() {
    if (!hasClaudeAi() || checking()) return;
    setChecking(true);
    try { recordConnectorChecks(await api.checkConnectors()); }
    catch (e) { flashFooter(`× ${errText(e)}`); }
    finally { setChecking(false); }
  }
  /** Hollow's own Microsoft sign-in: signed in, still valid, which scopes. */
  async function checkM365() {
    if (!m365Row() || m365Checking()) return;
    setM365Checking(true);
    try { recordM365Check(await api.m365Status()); }
    catch (e) { flashFooter(`× ${errText(e)}`); }
    finally { setM365Checking(false); }
  }
  const checkAll = () => Promise.all([checkStatus(), checkM365()]);
  // Login override: the Always warning matters only once a Claude account is
  // added (best effort; no warning when accounts can't be read).
  const [hasAdded, setHasAdded] = createSignal(false);
  onMount(async () => {
    api.claudeAccounts().then(v => setHasAdded(v.accounts.length > 0), () => setHasAdded(false));
    await load(); void checkAll();
  });

  async function update(c: ConnectorView, patch: Partial<ConnectorInput>) {
    try { await api.saveConnector(inputFrom(c, patch)); await load(); }
    catch (e) { flashFooter(`× ${errText(e)}`); }
  }
  async function remove(c: ConnectorView) {
    const ok = await showConfirm({
      title: `Remove ${c.name}?`,
      body: 'Agents stop getting it from the next run. Nothing changes on claude.ai or in Claude Code or Codex.',
      confirmLabel: 'Remove', danger: true,
    });
    if (!ok) return;
    try { await api.deleteConnector(c.id); if (open() === c.id) setOpen(null); await load(); } catch (e) { flashFooter(`× ${errText(e)}`); }
  }
  async function add(d: DiscoveredConnector) {
    if (adding()) return;
    setAdding(d.name);
    try {
      const v = await trackTask(`Add ${d.name}`, () => api.importConnector(d.source, d.name));
      await load();
      setCatalog(false);
      setOpen(v.id);
      flashFooter(`${d.name} added for ${d.source === 'claudeai' ? 'Claude' : 'Claude and Codex'}.`);
      void checkStatus();
    } catch (e) { flashFooter(`× ${errText(e)}`); }
    finally { setAdding(null); }
  }
  async function signIn() {
    await openInBrowser(CLAUDE_AI_CONNECTORS_URL);
    flashFooter('Sign in on claude.ai, then choose Check again.', 4000);
  }
  // The built-in Microsoft 365 connector: Hollow's own Microsoft sign-in.
  const [m365Busy, setM365Busy] = createSignal(false);
  async function m365SignIn(calendarOnly = false) {
    if (m365Busy()) return;
    setM365Busy(true);
    flashFooter('Finish signing in to Microsoft in your browser.', 5000);
    try {
      const s = await trackTask('Sign in to Microsoft 365', () => api.m365SignIn(calendarOnly));
      recordM365Check(s);
      await load();
      flashFooter(`Microsoft 365 connected${s.account ? ` as ${s.account}` : ''}${s.files ? '' : ' (calendar only)'}.`);
    } catch (e) { flashFooter(`× ${errText(e)}`, 8000); }
    finally { setM365Busy(false); }
  }
  async function m365SignOut() {
    const ok = await showConfirm({ title: 'Sign Microsoft 365 out?', body: 'Agents stop getting the built-in Microsoft 365 connector until you sign in again. Nothing changes on claude.ai.', confirmLabel: 'Sign out', danger: true });
    if (!ok) return;
    try { await api.m365SignOut(); recordM365Check({ signedIn: false, account: null, files: false, problem: null }); await load(); } catch (e) { flashFooter(`× ${errText(e)}`); }
  }
  // The built-in Plaud connector: Plaud's MCP server's own sign-in (overnight S8).
  const [plaudBusy, setPlaudBusy] = createSignal(false);
  async function plaudSignIn() {
    if (plaudBusy()) return;
    setPlaudBusy(true);
    flashFooter('Finish signing in to Plaud in your browser.', 5000);
    try {
      const s = await trackTask('Sign in to Plaud', () => api.plaudMcpSignIn());
      await load();
      flashFooter(s.already ? 'Plaud was already signed in, so nothing opened. It works on every Claude account and in Codex.' : 'Plaud connected. It works on every Claude account and in Codex.');
    } catch (e) { flashFooter(`× ${errText(e)}`, 8000); await load(); }
    finally { setPlaudBusy(false); }
  }
  async function plaudSignOut() {
    const ok = await showConfirm({ title: 'Sign Plaud out?', body: 'Agents stop getting the built-in Plaud connector until you sign in again. Plaud’s MCP server forgets its sign-in; Recordings keep theirs.', confirmLabel: 'Sign out', danger: true });
    if (!ok) return;
    try { await api.plaudMcpSignOut(); await load(); } catch (e) { flashFooter(`× ${errText(e)}`, 8000); }
  }
  // Remote connectors on Hollow's own sign-in (five asks B2).
  const [remoteBusy, setRemoteBusy] = createSignal<string | null>(null);
  async function remoteSignIn(c: ConnectorView) {
    if (remoteBusy()) return;
    setRemoteBusy(c.id);
    flashFooter(`Finish signing in to ${c.name} in your browser.`, 5000);
    try {
      await trackTask(`Sign in to ${c.name}`, () => api.remoteConnectorSignIn(c.id));
      await load();
      flashFooter(`${c.name} connected. It works on every Claude account and in Codex.`);
    } catch (e) { flashFooter(`× ${errText(e)}`, 8000); await load(); }
    finally { setRemoteBusy(null); }
  }
  async function remoteSignOut(c: ConnectorView) {
    const ok = await showConfirm({ title: `Sign ${c.name} out?`, body: `Agents stop getting ${c.name} until you sign in again. Nothing changes on claude.ai or in ${c.name}.`, confirmLabel: 'Sign out', danger: true });
    if (!ok) return;
    try { await api.remoteConnectorSignOut(c.id); await load(); } catch (e) { flashFooter(`× ${errText(e)}`); }
  }
  /** A claude.ai connector → the same service on Hollow's own sign-in. */
  async function useHollowSignIn(c: ConnectorView) {
    const found = hollowSignInFor(c, connectors());
    if (!found) return;
    if (found.kind === 'existing') { setOpen(found.id); return; }
    try {
      const v = await api.connectorUseHollowSignIn(c.id);
      await load();
      setOpen(v.id);
      await remoteSignIn(v);
    } catch (e) { flashFooter(`× ${errText(e)}`, 8000); }
  }
  /** The repair a row's state calls for (connectorCapability chooses it). */
  function repair(c: ConnectorView, kind: RepairKind) {
    if (kind === 'signInHollow') void remoteSignIn(c);
    else if (kind === 'signInClaudeAi') void signIn();
    else if (kind === 'checkAgain') void (c.transport === 'builtin' ? checkM365() : checkStatus());
    else if (kind === 'signInM365') void m365SignIn();
    else if (kind === 'grantM365') void m365SignIn(false);
    else if (kind === 'signInPlaud') void plaudSignIn();
    else if (kind === 'checkPlaud') void load();
  }
  const repairBusy = (c: ConnectorView, kind: RepairKind) => kind === 'checkAgain' ? (c.transport === 'builtin' ? m365Checking() : checking())
    : kind === 'signInHollow' ? remoteBusy() === c.id
    : kind === 'signInPlaud' ? plaudBusy()
    : (kind === 'signInM365' || kind === 'grantM365') && m365Busy();

  function edit(c?: ConnectorView) {
    setDraft(c ? {
      id: c.id, name: c.name, description: c.description, transport: c.transport,
      command: c.command, args: c.args.join('\n'), url: c.url,
      secrets: (c.transport === 'http' ? c.headerKeys : c.envKeys).map(key => ({ key, value: '', saved: true })),
      oauth: !!c.oauth, oauthClientId: c.oauthClientId ?? '',
      oauthClientSecret: '', secretSaved: !!c.oauthClientSecretSet, clearSecret: false,
      savedClientId: c.oauthClientId ?? '', savedUrl: c.url,
    } : { ...EMPTY, secrets: [] });
  }
  // 10-11 B4: Superhuman Mail on Hollow's own sign-in, one click: shared with
  // Claude and Codex, attached when relevant, and every tool that isn't a
  // read asks on Hollow's card (`remote_mcp`). Signing in stays the person's.
  const hasSuperhuman = () => connectors().some(c => c.transport === 'http' && c.oauth && approvedMail(c.url));
  async function addSuperhuman() {
    if (saving()) return;
    setSaving(true);
    try {
      // A free id: never an edit of a connector the library already has
      // (a Codex-imported `superhuman` stdio row, say).
      const taken = new Set(connectors().map(c => c.id));
      const id = ['superhuman_mail', ...Array.from({ length: 8 }, (_, i) => `superhuman_mail_${i + 2}`)].find(x => !taken.has(x)) ?? `superhuman_mail_${Date.now()}`;
      const v = await api.saveConnector({
        id, name: 'Superhuman Mail', description: 'Your mail as context: search and read threads. Sending, drafts and changes ask first.',
        transport: 'http', command: '', args: [], url: SUPERHUMAN_URL, env: {}, headers: {},
        oauth: true, oauthClientId: '', agents: ['claude', 'codex'] as SharedAgent[], chat: true, automations: false,
        // Reads run without a prompt in Edits only too; every write still asks
        // on Hollow's card, whatever this switch says.
        mode: 'relevant', autoApprove: true, enabled: true,
      } as ConnectorInput);
      await load(); setCatalog(false); setOpen(v.id);
    } catch (e) { flashFooter(`× ${errText(e)}`); }
    finally { setSaving(false); }
  }
  async function saveDraft() {
    const d = draft(); if (!d || saving()) return;
    const existing = connectors().find(c => c.id === d.id);
    const secrets = secretsFrom(d.secrets);
    const input: ConnectorInput = {
      ...(existing ? inputFrom(existing) : d.preset ? SERVICE_SETUP_DEFAULTS : {
        agents: ['claude', 'codex'] as SharedAgent[], chat: true, automations: false, mode: 'always' as const, autoApprove: true, enabled: true,
      }),
      id: d.id, name: d.name, description: d.description, transport: d.transport,
      command: d.command, args: parseArgs(d.args), url: d.url, serverId: existing?.serverId,
      env: d.transport === 'stdio' ? secrets : {}, headers: d.transport === 'http' && !d.oauth ? secrets : {},
      oauth: d.transport === 'http' && d.oauth, oauthClientId: d.transport === 'http' && d.oauth ? d.oauthClientId.trim() : '',
      ...clientSecretInput(d),
    } as ConnectorInput;
    setSaving(true);
    try { const v = await api.saveConnector(input); setDraft(null); await load(); setOpen(v.id); }
    catch (e) { flashFooter(`× ${errText(e)}`); }
    finally { setSaving(false); }
  }
  const patchDraft = (p: Partial<Draft>) => setDraft(d => (d ? { ...d, ...p } : d));
  /** What saving does to the saved client secret (B2, #537 review). */
  const secretFate = () => { const d = draft(); return d ? savedSecretFate(d, { clientId: d.savedClientId, url: d.savedUrl }) : 'none'; };

  const available = createMemo(() => (view()?.discovered ?? []).filter(d => !d.inLibrary));
  const shown = createMemo(() => available().filter(d => matchesQuery(d, query())));
  const needsSignIn = createMemo(() => connectors().filter(c => c.transport === 'claudeAi' && capOf(c).state === 'needsSignIn').length);

  return <section ref={root} class="cx" aria-label="Connectors">
    <section class="cx-block" aria-label="Services">
      {/* Settings PR 5b: two one-line rows. Calendar's own row is the one below
          (Agenda calendar), so it needs no card pointing at it. */}
      <div class="cx-block-head"><h3>Services</h3>
        <details class="cx-service-boundaries"><summary>About account access</summary><p>Connections in other apps do not grant Hollow access. A connection signed in here can be shared with the supported agents listed on its row. Review service access at sign-in and tool permissions in the connection settings.</p></details>
      </div>
      <div class="cx-service-grid" data-jump-skip>
        <div class="cx-service"><strong>GitHub</strong><p>Repository, issue and pull-request context, read-only to start.</p><button type="button" class="btn small ghost" data-sheet-opener="connector-github" disabled={!view() || !!error() || saving()} onClick={e => { opener.mark(e, '[data-sheet-opener="connector-github"]'); void setupGitHub(); }}>{githubConnection(connectors()) ? 'Manage GitHub' : 'Set up GitHub'}</button></div>
      </div>
      <CalendarSource/>
    </section>
    <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    <For each={view()?.problems ?? []}>{p => <p class="work-error">{p}</p>}</For>
    <Show when={view()} fallback={<Skeleton label="Loading connectors" rows={4} />}>
      {/* ── Your connectors ─────────────────────────────────────── */}
      <div class="cx-block">
        <div class="cx-block-head">
          <div>
            <h3>Your connectors</h3>
            <LeadText text="Apps your agents can use in chats and automations. Switch one off to hide it." />
          </div>
          <Show when={hasClaudeAi() || m365Row()}>
            <button class="btn small ghost" disabled={checking() || m365Checking()} onClick={() => void checkAll()}>
              <Icon name="refresh" size={12} /> {checking() || m365Checking() ? 'Checking…' : 'Check again'}
            </button>
          </Show>
        </div>
        <Show when={needsSignIn()}>
          <div class="cx-banner" role="status">
            <Icon name="alert" size={14} />
            <span>{needsSignIn()} connector{needsSignIn() === 1 ? '' : 's'} need{needsSignIn() === 1 ? 's' : ''} signing in on claude.ai before agents can use {needsSignIn() === 1 ? 'it' : 'them'}.<Show when={view()!.claudeaiLogin}>{login => <> Agents use them as <strong>{describeLogin(login())}</strong>, Claude Code's login. Connected under another claude.ai account or organization doesn't count.</>}</Show></span>
            <button class="btn small primary" onClick={() => void signIn()}>Open claude.ai</button>
          </div>
        </Show>
        <div class="cx-list">
          <For each={connectors()} fallback={<div class="cx-empty">No connectors yet. Add one from the list below.</div>}>
            {c => {
              const info = () => capOf(c);
              const isOpen = () => open() === c.id;
              return <article id={`connector-${c.id}`} class="cx-row" data-open={isOpen() ? 'true' : 'false'} data-enabled={c.enabled ? 'on' : 'off'}>
                <div class="cx-row-main">
                  <button class="cx-row-toggle" aria-expanded={isOpen()} aria-label={`${c.name} settings`} onClick={() => setOpen(isOpen() ? null : c.id)}>
                    <ConnectorIcon name={c.name} src={iconFor(c)} />
                    <span class="cx-row-text">
                      <strong>{c.name}</strong>
                      <small>{info().line}</small>
                    </span>
                  </button>
                  <span class="cx-pill" data-tone={info().tone} data-state={info().state} title={`Tested: ${info().tested}`}>{info().label}</span>
                  <Show when={info().repair}>{r =>
                    <button class="btn small" title={r().hint} disabled={repairBusy(c, r().kind)} onClick={() => repair(c, r().kind)}>
                      {repairBusy(c, r().kind) ? (r().kind === 'checkAgain' ? 'Checking…' : 'Signing in…') : r().label}
                    </button>
                  }</Show>
                  <Switch checked={c.enabled} label={`${c.enabled ? 'Turn off' : 'Turn on'} ${c.name}`} onChange={on => update(c, { enabled: on })} />
                  <button class="cx-chevron" aria-hidden="true" tabIndex={-1} onClick={() => setOpen(isOpen() ? null : c.id)}>
                    <Show when={isOpen()} fallback={<Icon name="chevronRight" size={14} />}><Icon name="chevronDown" size={14} /></Show>
                  </button>
                </div>
                <Show when={alwaysLoginWarning(c, hasAdded(), connectors())}>{warning =>
                  <div class="cx-banner" role="status" data-kind="login-override">
                    <Icon name="alert" size={14} />
                    <span>{warning()}</span>
                    <button class="btn small" onClick={() => void update(c, { mode: 'relevant' })}>Attach only when relevant</button>
                  </div>
                }</Show>
                <Show when={isOpen()}>
                  <div class="cx-detail">
                    {/* Settings PR 5: the row is one line; what Hollow tested and where it applies show here. */}
                    <Show when={c.enabled}><small class="cx-evidence">{evidenceLine(info())} · {info().appliesTo}</small></Show>
                    <Show when={c.enabled}><small class="cx-evidence" data-kind="works-on">{worksOn(c, connectors())}</small></Show>
                    <Show when={c.description}><p class="cx-desc">{c.description}</p></Show>
                    <dl class="cx-facts">
                      <dt>Source</dt><dd>{c.transport === 'builtin' ? 'Built into Hollow' : c.transport === 'claudeAi' ? 'claude.ai' : c.transport === 'http' ? (c.oauth ? 'Remote server, on Hollow’s own sign-in' : 'Remote server') : 'Local command'}{c.origin !== 'hollow' && c.transport !== 'claudeAi' ? ` · from ${SOURCE_NAMES[c.origin] ?? c.origin}` : ''}</dd>
                      <Show when={c.transport !== 'builtin'}><dt>{c.transport === 'stdio' ? 'Command' : 'Address'}</dt><dd class="cx-mono">{c.transport === 'stdio' ? [c.command, ...c.args].join(' ') : c.url}</dd></Show>
                      <Show when={c.envKeys.length + c.headerKeys.length}><dt>Secrets</dt><dd class="cx-mono">{[...c.envKeys, ...c.headerKeys].join(', ')}</dd></Show>
                      <dt>Status</dt><dd>{info().label}. {info().line}</dd>
                      <dt>Tested</dt><dd>{info().tested}</dd>
                      <dt>Last check</dt><dd>{evidenceLine(info())}</dd>
                      <Show when={info().missing}><dt>Missing</dt><dd>{info().missing}</dd></Show>
                      <dt>Applies to</dt><dd>{info().appliesTo}</dd>
                      <dt>Works on</dt><dd>{worksOn(c, connectors()).replace(/^Works on: /, '')}</dd>
                      <Show when={statuses()[c.id]?.detail}><dt>claude.ai says</dt><dd>{statuses()[c.id]!.detail}</dd></Show>
                    </dl>
                    <div class="cx-options">
                      <div class="cx-option">
                        <span class="cx-option-k">Agents</span>
                        <div class="cx-chips" role="group" aria-label={`Agents sharing ${c.name}`}>
                          <For each={shareableAgents(c.transport)}>{a => <label class="cx-chip"><input type="checkbox" checked={c.agents.includes(a)} onChange={() => update(c, { agents: toggleAgent(c.agents, a) })} />{AGENT_NAMES[a]}</label>}</For>
                        </div>
                        <Show when={c.transport === 'claudeAi'}><small>Claude only. Runs that use it go out on your Claude Code login{view()?.claudeaiLogin ? ` (${describeLogin(view()!.claudeaiLogin!)})` : ''}, the account it belongs to.<Show when={hollowSignInFor(c, connectors())}> With <strong>Use Hollow’s sign-in</strong>, Hollow signs in to it itself, so it works on every Claude account and in Codex.</Show></small></Show>
                      </div>
                      <div class="cx-option">
                        <span class="cx-option-k">Use in</span>
                        <div class="cx-chips" role="group" aria-label={`Where ${c.name} is used`}>
                          <label class="cx-chip"><input type="checkbox" checked={c.chat} onChange={() => update(c, { chat: !c.chat })} />Chats</label>
                          <label class="cx-chip"><input type="checkbox" checked={c.automations} onChange={() => update(c, { automations: !c.automations })} />Automations</label>
                        </div>
                      </div>
                      <div class="cx-option">
                        <span class="cx-option-k">Attach</span>
                        <div class="cx-seg" role="radiogroup" aria-label={`When to attach ${c.name}`}>
                          <button role="radio" aria-checked={c.mode === 'always'} onClick={() => update(c, { mode: 'always' })}>Always</button>
                          <button role="radio" aria-checked={c.mode === 'relevant'} onClick={() => update(c, { mode: 'relevant' })}>Only when relevant</button>
                        </div>
                        <small>“Only when relevant” attaches it when the request seems to need it, which keeps other runs lighter.</small>
                        <Show when={alwaysSpreadHint(c, hasAdded(), connectors())}>{hint => <small data-kind="always-spread">{hint()}</small>}</Show>
                      </div>
                      <div class="cx-option cx-option-inline">
                        <span class="cx-option-k">{AUTO_APPROVE_LABEL}</span>
                        <Switch checked={c.autoApprove} label={`Run ${c.name} tools without asking in Edits only`} onChange={on => update(c, { autoApprove: on })} />
                        <small>{AUTO_APPROVE_HINT}</small>
                      </div>
                      <Show when={hollowEquivalent(c, connectors())}>{eq =>
                        <div class="cx-option cx-option-inline">
                          <span class="cx-option-k">Use Hollow’s instead</span>
                          <Switch checked={!c.keepClaudeai} label={`Use Hollow’s ${eq().name} instead of ${c.name}`} onChange={on => update(c, { keepClaudeai: !on })} />
                          <small>On: Hollow’s own {eq().name} is used once it is signed in, on every Claude account and in Codex. Off: this claude.ai one is used, on Claude Code’s login only, and Hollow’s is left out.</small>
                        </div>
                      }</Show>
                    </div>
                    <div class="cx-actions">
                      <Show when={c.transport !== 'claudeAi' && c.transport !== 'builtin'}><button class="btn small ghost" data-sheet-opener="connector-edit" onClick={e => { opener.mark(e, `#connector-${c.id} [data-sheet-opener="connector-edit"]`); edit(c); }}>Edit</button></Show>
                      <Show when={c.transport === 'claudeAi' && hollowSignInFor(c, connectors())}>{found =>
                        <button class="btn small" disabled={remoteBusy() !== null} onClick={() => void useHollowSignIn(c)}>{found().kind === 'existing' ? 'Show Hollow’s sign-in' : 'Use Hollow’s sign-in'}</button>
                      }</Show>
                      <Show when={c.transport === 'claudeAi'}><button class="btn small ghost" onClick={() => void openInBrowser(CLAUDE_AI_CONNECTORS_URL)}><Icon name="externalLink" size={12} /> Manage on claude.ai</button></Show>
                      <Show when={c.transport === 'http' && c.oauth && c.remote?.signedIn}>
                        <button class="btn small ghost" disabled={remoteBusy() !== null} onClick={() => void remoteSignIn(c)}>Sign in again</button>
                        <button class="btn small ghost cx-remove" onClick={() => void remoteSignOut(c)}>Sign out</button>
                      </Show>
                      <Show when={c.builtin?.kind === 'm365'}>
                        {/* A sign-in Microsoft refused is not ready but still signed in: offer Sign out (review B3 #5). */}
                        <Show when={c.builtin!.ready || m365Check()?.signedIn} fallback={<button class="btn small ghost" disabled={m365Busy()} onClick={() => void m365SignIn(true)}>Sign in for calendar only</button>}>
                          <button class="btn small ghost" disabled={m365Busy()} onClick={() => void m365SignIn()}>Sign in again</button>
                          <button class="btn small ghost cx-remove" onClick={() => void m365SignOut()}>Sign out</button>
                        </Show>
                      </Show>
                      {/* Signed in: Sign out only (its login tool would just say "Already logged in."); signed out: the row's Sign in. */}
                      <Show when={c.builtin?.kind === 'plaud' && c.builtin.ready}>
                        <button class="btn small ghost cx-remove" disabled={plaudBusy()} onClick={() => void plaudSignOut()}>Sign out</button>
                      </Show>
                      <Show when={c.transport !== 'builtin'}><button class="btn small ghost cx-remove" onClick={() => remove(c)}>Remove</button></Show>
                    </div>
                  </div>
                </Show>
              </article>;
            }}
          </For>
        </div>
      </div>

      {/* ── Custom connector form ───────────────────────────────── */}
      <Show when={draft()}>
        {d => <SettingsSheet id="settings-sheet-connector" title={d().id ? 'Edit a connector' : d().preset ? 'Connect GitHub' : 'Add a custom connector'} returnTo={opener.get} locked={saving} onClose={() => setDraft(null)}><form class="cx-form" aria-label={d().id ? `Edit ${d().name}` : 'New connector'} onSubmit={e => { e.preventDefault(); void saveDraft(); }}>
          <Show when={d().preset}><p class="cx-form-hint">{GITHUB_PRESET.setup}</p><p class="cx-form-hint">Shared with Claude and Codex · attach only when relevant · automations off · run-without-asking off. Review these preferences after saving.</p></Show>
          <p class="cx-form-hint">Any MCP server: a remote URL, or a command that runs on this Mac.</p>
          <label>Name<input aria-label="Connector name" value={d().name} onInput={e => patchDraft({ name: e.currentTarget.value })} placeholder="Linear" required /></label>
          <label>What it's for<input aria-label="Connector description" value={d().description} onInput={e => patchDraft({ description: e.currentTarget.value })} placeholder="Read and update issues" /><small>Helps Hollow decide when it's relevant.</small></label>
          <div class="cx-seg" role="radiogroup" aria-label="How it runs">
            <button type="button" role="radio" aria-checked={d().transport === 'http'} onClick={() => patchDraft({ transport: 'http' })}>Remote URL</button>
            <button type="button" role="radio" aria-checked={d().transport === 'stdio'} onClick={() => patchDraft({ transport: 'stdio' })}>Local command</button>
          </div>
          <Show when={d().transport === 'stdio'} fallback={<label>URL<input aria-label="Connector URL" value={d().url} onInput={e => patchDraft({ url: e.currentTarget.value })} placeholder="https://mcp.example.com/mcp" /></label>}>
            <label>Command<input aria-label="Connector command" value={d().command} onInput={e => patchDraft({ command: e.currentTarget.value })} placeholder="npx" /></label>
            <label>Arguments<textarea aria-label="Connector arguments" rows={2} value={d().args} onInput={e => patchDraft({ args: e.currentTarget.value })} placeholder={'-y\n@example/mcp-server'} /><small>One per line.</small></label>
          </Show>
          <Show when={!(d().transport === 'http' && d().oauth)}><fieldset class="cx-secrets">
            <legend>{d().transport === 'http' ? 'Headers' : 'Environment variables'} <small>stored privately on this Mac, never shown again</small></legend>
            <Index each={d().secrets}>{(row, i) => <div class="cx-secret-row">
              <input aria-label="Name" value={row().key} disabled={row().saved} onInput={e => patchDraft({ secrets: d().secrets.map((r, j) => j === i ? { ...r, key: e.currentTarget.value } : r) })} placeholder={d().transport === 'http' ? 'Authorization' : 'API_KEY'} />
              <input aria-label="Value" type="password" value={row().value} onInput={e => patchDraft({ secrets: d().secrets.map((r, j) => j === i ? { ...r, value: e.currentTarget.value } : r) })} placeholder={row().saved ? 'Saved; type to replace' : 'Value'} />
              <button type="button" class="btn small ghost" aria-label="Remove" onClick={() => patchDraft({ secrets: d().secrets.filter((_, j) => j !== i) })}>×</button>
            </div>}</Index>
            <button type="button" class="btn small ghost" onClick={() => patchDraft({ secrets: [...d().secrets, { key: '', value: '', saved: false }] })}>Add {d().transport === 'http' ? 'header' : 'variable'}</button>
          </fieldset></Show>
          <Show when={secretFate() === 'dropped'}><p class="cx-form-hint" role="status">The saved client secret goes when you save (a new client id or address, or no app sign-in), and the connector signs out; type the secret again to keep one.</p></Show>
          <Show when={d().transport === 'http'}>
            <label class="cx-check"><input type="checkbox" checked={d().oauth} onChange={e => patchDraft({ oauth: e.currentTarget.checked })} />Sign in with Hollow<small>Hollow signs in to the server once (OAuth), so it works on every Claude account and in Codex.</small></label>
            <Show when={d().oauth} fallback={<p class="cx-form-hint">If the server needs a sign-in, each agent asks for it once: in Claude Code run <code>/mcp</code>, in Codex <code>codex mcp login &lt;id&gt;</code>. Or tick <strong>Sign in with Hollow</strong>.</p>}>
              <label>Client id <small>optional</small><input aria-label="OAuth client id" value={d().oauthClientId} onInput={e => patchDraft({ oauthClientId: e.currentTarget.value })} placeholder="Only if the server can't register Hollow itself" /><small>For an app you registered with the server; its redirect address must be <code>http://127.0.0.1:53683/callback</code>.</small></label>
              <Show when={d().oauthClientId.trim()}>
                <div class="cx-secret-field">
                  <label>Client secret <small>optional, stored privately on this Mac, never shown again</small><input aria-label="OAuth client secret" type="password" autocomplete="off" value={d().oauthClientSecret} onInput={e => patchDraft({ oauthClientSecret: e.currentTarget.value, clearSecret: false })} placeholder={secretFate() === 'kept' ? 'Saved; type to replace' : 'Only for an app the server treats as confidential (Zoom Marketplace apps are)'} /></label>
                  <Show when={secretFate() === 'kept'}><button type="button" class="btn small ghost" onClick={() => patchDraft({ clearSecret: true })}>Remove secret</button></Show>
                  <Show when={secretFate() === 'cleared' || secretFate() === 'replaced'}><small>Saving signs the connector out; sign in again after.</small></Show>
                </div>
              </Show>
            </Show>
          </Show>
          <div class="settings-actions"><button class="btn primary" type="submit" disabled={saving() || !d().name.trim()}>{saving() ? 'Saving…' : 'Save connector'}</button><button class="btn ghost" type="button" onClick={() => setDraft(null)}>Cancel</button></div>
        </form></SettingsSheet>}
      </Show>

      {/* ── Add connectors: a row; the catalog is a sheet (Settings PR 5) ── */}
      <div class="cx-block cx-add-row">
        <div class="cx-block-head">
          <div>
            <h3>Add connectors</h3>
            <p>Found on claude.ai and in Claude Code or Codex, plus GitHub, Superhuman Mail and any MCP server.</p>
          </div>
          <button class="btn small" data-sheet-opener="connector-add" onClick={e => { opener.mark(e); setCatalog(true); }}>Add a connector…</button>
        </div>
      </div>
      <Show when={catalog() && !draft()}>
        <SettingsSheet id="settings-sheet-catalog" title="Add a connector" returnTo={opener.get} locked={() => adding() !== null || saving()} onClose={() => { setCatalog(false); setQuery(''); }}>
      <div class="cx-block cx-catalog">
        <div class="cx-block-head">
          <div>
            <p>Found on your claude.ai account and in Claude Code or Codex. claude.ai ones work with Claude; the others with Claude and Codex.</p>
          </div>
          <button class="btn small ghost" onClick={() => void load(true).then(() => checkStatus())}><Icon name="refresh" size={12} /> Refresh</button>
        </div>
        <Show when={available().length > 6}>
          <input class="cx-search" type="search" aria-label="Search connectors" placeholder="Search connectors" value={query()} onInput={e => setQuery(e.currentTarget.value)} />
        </Show>
        <div class="cx-gallery">
          <For each={shown()}>{d => <div class="cx-tile" data-unsupported={d.unsupported ? 'true' : 'false'}>
            <ConnectorIcon name={d.name} src={iconFor(d)} />
            <span class="cx-tile-text">
              <strong>{d.name}</strong>
              <small>{SOURCE_NAMES[d.source] ?? d.source}</small>
              <Show when={d.unsupported ?? d.note}>{t => <small class="cx-tile-note">{t()}</small>}</Show>
            </span>
            <Show when={!d.unsupported}>
              <button class="btn small" aria-label={`Add ${d.name}`} disabled={adding() !== null} onClick={() => void add(d)}>{adding() === d.name ? 'Adding…' : 'Add'}</button>
            </Show>
          </div>}</For>
          <Show when={!hasSuperhuman()}>
            <div class="cx-tile">
              <ConnectorIcon name="Superhuman Mail" src={iconFor({ name: 'Superhuman Mail', url: SUPERHUMAN_URL, transport: 'http' })} />
              <span class="cx-tile-text">
                <strong>Superhuman Mail on Hollow’s sign-in</strong>
                <small>For Claude and Codex</small>
                <small class="cx-tile-note">Reads your mail when relevant; sending and changes ask first</small>
              </span>
              <button class="btn small" aria-label="Add Superhuman Mail on Hollow’s sign-in" disabled={saving()} onClick={() => void addSuperhuman()}>Add</button>
            </div>
          </Show>
          <Show when={!githubConnection(connectors())}>
            <div class="cx-tile">
              <span class="cx-icon cx-icon-md" aria-hidden="true"><Icon name="plus" size={16} /></span>
              <span class="cx-tile-text"><strong>GitHub</strong><small>Repository, issue and pull-request context</small><small class="cx-tile-note">Starts with the read-only server</small></span>
              <button class="btn small" aria-label="Add GitHub" disabled={!view() || !!error() || saving()} onClick={() => { setCatalog(false); void setupGitHub(); }}>Add</button>
            </div>
          </Show>
          <button class="cx-tile cx-tile-custom" onClick={() => { setCatalog(false); edit(); }}>
            <span class="cx-icon cx-icon-md cx-icon-plus" aria-hidden="true"><Icon name="plus" size={16} /></span>
            <span class="cx-tile-text"><strong>Custom connector</strong><small>Any MCP server URL or command</small></span>
          </button>
        </div>
        <Show when={query() && !shown().length}><p class="cx-empty">Nothing matches “{query()}”.</p></Show>
        <Show when={view()!.claudeaiError}><p class="cx-note">Your claude.ai connectors couldn't be listed: {view()!.claudeaiError}</p></Show>
      </div>
        </SettingsSheet>
      </Show>
    </Show>
  </section>;
}
