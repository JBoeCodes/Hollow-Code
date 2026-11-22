import { createSignal, createResource, onMount, Show, For } from 'solid-js';
import { api } from '../api';
import { Switch } from '../components/Switch';
import { requestOpenSettingsSection, trackTask, chatAgentSessionKey } from '../store';
import type { JevDecisionsReport, JevStatus, RunContextView } from '../types';
import { toolUseRows, largestRows } from '../chatUsage';
import { contextReportLine } from './contextReport';
import '../styles/connectors-library.css';
import { ErrorText } from '../components/ui';
import { MoreFold } from './settingsLayout';

function errText(e: unknown) { return e instanceof Error ? e.message : String(e); }

/** Settings, Advanced, Search & indexes, Context prioritizer: independent Jev ranking and
 *  duplicate-check preferences (Rust `jev.rs` / `jev_decide.rs`). Each is off
 *  by default, and Hollow works fully with both off or with no key at all. The key is managed in
 *  Settings, Accounts and lives in the macOS
 *  Keychain; this panel only ever sees whether one is saved. */
export default function ContextPrioritizer() {
  const [conversation] = createResource(chatAgentSessionKey, async key => {
    const [manifest, use] = await Promise.all([api.runContextManifest(null, key).catch(() => null), api.conversationContextUse(key)]);
    return { manifest, use };
  });
  const [status, setStatus] = createSignal<JevStatus | null>(null);
  const [report, setReport] = createSignal<RunContextView | null>(null);
  const [decisions, setDecisions] = createSignal<JevDecisionsReport | null>(null);
  const [key, setKey] = createSignal('');
  const [message, setMessage] = createSignal('');
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  async function load() {
    try { setStatus(await api.jevStatus()); setError(''); } catch (e) { setError(errText(e)); }
    // The newest run's own manifest (agent effectiveness F11), not a
    // process-wide "last report" another turn could overwrite.
    try { setReport(await api.runContextManifest()); } catch { /* report is optional */ }
    try { setDecisions(await api.jevDecisionsReport()); } catch { /* report is optional */ }
  }
  onMount(load);
  async function run(label: string, fn: () => Promise<unknown>, done?: string) {
    setBusy(true); setError(''); setMessage('');
    try { await trackTask(label, fn); if (done) setMessage(done); await load(); }
    catch (e) { setError(errText(e)); } finally { setBusy(false); }
  }
  const save = (enabled: boolean) => run('Save Jev settings', async () => { setStatus(await api.saveJevSettings(enabled, key().trim() || null)); setKey(''); }, enabled ? 'Jev is on for new requests.' : 'Saved.');
  return <div class="context-prioritizer">
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">Jev relevance model <span class="settings-state-chip" data-state={status()?.enabled ? 'ok' : undefined}>{status()?.enabled ? 'on' : status()?.keyPresent ? 'key saved, off' : 'off'}</span></div>
        <div class="settings-row-v">Optional, off by default. Uses your TypeSafe key to rank context for each request.</div>
        {/* Risky: text leaves this Mac, so what is sent stays one click away. */}
        <MoreFold><div class="settings-row-v">Jev scores the candidate passages and connectors: unrelated passages are left out and the rest ordered by usefulness. Only your request text and short excerpts are sent (never whole files, paths or the conversation), billed to your TypeSafe account. If Jev is slow or unavailable, Hollow falls back to local prioritizing for that request.</div></MoreFold>
        <Show when={status() && !status()!.keyPresent}><div class="settings-note">Needs a Jev key: <button type="button" class="btn-link" onClick={() => requestOpenSettingsSection('accounts', 'Jev (TypeSafe)')}>add in Accounts</button></div></Show>
      </div>
      <Switch checked={Boolean(status()?.enabled)} label="Use Jev to rank context" disabled={busy() || !status()?.keyPresent} onChange={next => save(next)} />
    </div>
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">Jev duplicate check <span class="settings-state-chip" data-state={status()?.decisionsEnabled ? 'ok' : undefined}>{status()?.decisionsEnabled ? 'on' : 'off'}</span></div>
        <div class="settings-row-v">Optional, off by default. Checks suggestions for repeats before they reach review.</div>
        {/* Risky: titles leave this Mac, so what is sent stays one click away. */}
        <MoreFold><div class="settings-row-v">Jev decides whether a task or learning suggestion repeats something you already have, even when it is worded differently. Confident repeats are not queued; likely ones are queued with a “possibly the same as …” note. Sends the suggestion titles and the titles of a few similar tasks or learnings (never files, paths or the conversation), billed to your account. Without it, Hollow still skips exact repeats locally.</div></MoreFold>
        <Show when={status() && !status()!.keyPresent}><div class="settings-note">Needs a Jev key: <button type="button" class="btn-link" onClick={() => requestOpenSettingsSection('accounts', 'Jev (TypeSafe)')}>add in Accounts</button></div></Show>
      </div>
      <Switch checked={Boolean(status()?.decisionsEnabled)} label="Use Jev to check suggestions for duplicates" disabled={busy() || !status()?.keyPresent}
        onChange={() => run('Save duplicate check preference', async () => setStatus(await api.saveJevSettings(Boolean(status()?.enabled), null, !status()?.decisionsEnabled)), 'Duplicate check preference saved.')} />
    </div>
    <div class="settings-actions">
      <p class="settings-note">{status()?.keyPresent ? 'Jev key saved.' : 'No Jev key yet.'} Add, test or remove it in Settings → Accounts → Other providers.</p>
      {/* Lands on the Jev row itself, not the top of Accounts (P1-22). */}
      <button class="btn ghost" onClick={() => requestOpenSettingsSection('accounts', 'Jev (TypeSafe)')}>Manage key</button>
    </div>
    <Show when={message()}><p class="settings-note" role="status">{message()}</p></Show>
    <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    <Show when={!conversation.loading && !conversation.error && conversation()}>{c => <details class="settings-fold">
      <summary>Conversation context details</summary>
      <p class="settings-note">Recorded tool results since this agent session began; token estimates use bytes divided by four. {c().use.atLeast ? 'At least these results. ' : ''}{c().use.windowTurns - c().use.measuredTurns} turns unmeasured.</p>
      <dl><For each={[...toolUseRows(c().use), ...largestRows(c().use)]}>{r => <><dt>{r.label}</dt><dd>{r.value}</dd></>}</For></dl>
      <Show when={c().manifest}><details><summary>Recorded message layers and sources</summary><pre style={{ "max-height": "24rem", overflow: "auto", "white-space": "pre-wrap", "overflow-wrap": "anywhere" }}>{JSON.stringify(c().manifest!.manifest, null, 2)}</pre></details></Show>
    </details>}</Show>
    <Show when={report()}>{r => <p class="context-report">{contextReportLine(r())}</p>}</Show>
    <Show when={decisions()}>{d => <p class="context-report">Last duplicate check ({d().subject}, {d().engine === 'jev' ? 'Jev' : 'Jev unavailable'}): {d().checked} checked, {d().dropped} not queued, {d().flagged} flagged for review.{d().note ? ` ${d().note}` : ''}</p>}</Show>
  </div>;
}
