// Settings, Accounts, Agent updates: each agent CLI's installed and
// latest version, how it was installed, and its Updates choice. Updates use
// the install method's own updater (Rust `agent_cli`).
import { For, Show, createEffect, createResource, createSignal, onCleanup } from 'solid-js';
import { syncAgentHealthSupport } from '../store/agentHealth';
import { api } from '../api';
import { Select } from '../components/Select';
import { ErrorText } from '../components/ui';
import { flashFooter, showConfirm } from '../store';
import { errMsg } from '../utils';
import { UPDATE_POLICIES, knownBadLines, primaryAction, stateLine, updatableAgents, versionLine } from '../agentUpdates';
import type { AgentCliStatus } from '../generated/ipc';
import { unlessErrored } from '../stableResource';
import { ResourceReadError } from '../components/ResourceReadError';

export function AgentUpdates() {
  const [rows, { mutate, refetch }] = createResource(() => api.agentCliStatus(false));
  createEffect(() => { const current = unlessErrored(rows); if (current) syncAgentHealthSupport(current); });
  const [checking, setChecking] = createSignal(false);
  const [busy, setBusy] = createSignal<ReadonlySet<string>>(new Set());
  const isBusy = (agent: string) => busy().has(agent);
  const mark = (agent: string, on: boolean) => setBusy(prev => {
    const next = new Set(prev);
    if (on) next.add(agent); else next.delete(agent);
    return next;
  });
  const [error, setError] = createSignal('');
  const replace = (row: AgentCliStatus) => mutate(prev => prev?.map(r => (r.agent === row.agent ? row : r)));

  // Keep the rows current: every 5 s while something checks, updates or
  // waits, else every 30 s (the background worker may start one).
  let ticks = 0;
  const timer = setInterval(() => {
    ticks += 1;
    const active = unlessErrored(rows)?.some(r => r.state !== 'idle');
    if (busy().size === 0 && !rows.loading && (active || ticks % 6 === 0)) void refetch();
  }, 5000);
  onCleanup(() => clearInterval(timer));

  async function act(agent: string, run: () => Promise<AgentCliStatus | void>) {
    mark(agent, true);
    setError('');
    try {
      const row = await run();
      if (row) replace(row);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      mark(agent, false);
    }
  }

  async function checkNow() {
    setChecking(true);
    setError('');
    try {
      mutate(await api.agentCliStatus(true));
      flashFooter('Checked each agent for a newer version', 1800);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setChecking(false);
    }
  }

  /** Update, after a confirm when the version it installs is on Hollow's
   *  known-bad list (warn, never block; repo ideas 2026-10-27 #9). */
  async function update(row: AgentCliStatus, now = false) {
    if (row.latest && row.latestKnownBad) {
      const ok = await showConfirm({
        title: `Update ${row.label} to ${row.latest}?`,
        summary: `${row.latest} is on Hollow's list of versions known to break it.`,
        body: `${row.latestKnownBad}\n\nYou can update anyway; Restore can go back where the install method allows it.`,
        confirmLabel: 'Update anyway',
      });
      if (!ok) return;
    }
    await act(row.agent, () => api.updateAgentCli(row.agent, now));
  }

  /** Update every agent with an update to take, one after another; a failure
   *  is said once at the end and does not stop the rest. Known-bad versions
   *  stay out: they ask first, on their own row. */
  async function updateAll() {
    const list = updatableAgents((unlessErrored(rows) ?? []).filter(r => r.installed));
    if (!list.length) return;
    setError('');
    const failed: string[] = [];
    for (const row of list) {
      mark(row.agent, true);
      try { replace(await api.updateAgentCli(row.agent, false)); }
      catch (e) { failed.push(`${row.label}: ${errMsg(e)}`); }
      finally { mark(row.agent, false); }
    }
    if (failed.length) setError(failed.join(' '));
    else flashFooter(`Updating ${list.length === 1 ? list[0].label : `${list.length} agents`}`, 1800);
  }

  async function setPolicy(row: AgentCliStatus, policy: 'automatic' | 'ask' | 'off') {
    await act(row.agent, async () => {
      const saved = await api.setAgentCliPolicy(row.agent, policy);
      return { ...row, policy: saved };
    });
  }

  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-v">
            Automatic checks about once a day and updates an agent only while no run is using it.
          </div>
          <Show when={error()}>{msg => <ErrorText class="settings-row-warn" as="div" text={msg()} />}</Show>
          <ResourceReadError label="the agents" read={rows} retry={refetch} />
        </div>
        <div class="settings-actions">
          <button type="button" class="btn" disabled={checking() || rows.loading} onClick={() => void checkNow()}>
            {checking() ? 'Checking…' : 'Check now'}
          </button>
          <Show when={updatableAgents((unlessErrored(rows) ?? []).filter(r => r.installed)).length > 1}>
            <button type="button" class="btn" disabled={busy().size > 0} onClick={() => void updateAll()}>Update all</button>
          </Show>
        </div>
      </div>
      <For each={(unlessErrored(rows) ?? []).filter(row => row.installed)}>{agentRow}</For>
      {/* Agents not installed fold below the installed ones (P1-22). */}
      <Show when={(unlessErrored(rows) ?? []).filter(row => !row.installed)}>{missing =>
        <Show when={missing().length}>
          <details class="settings-fold settings-row-more">
            <summary>Not installed ({missing().length})</summary>
            <For each={missing()}>{agentRow}</For>
          </details>
        </Show>}
      </Show>
    </>
  );

  function agentRow(row: AgentCliStatus) {
        const action = () => primaryAction(row);
        const status = () => stateLine(row) ?? row.message;
        return (
          <div class="settings-row" data-agent={row.agent}>
            <div class="settings-row-label">
              <div class="settings-row-k">{row.label}</div>
              <div class="settings-row-v">{versionLine(row)}</div>
              <Show when={status()}>{text =>
                row.tone === 'danger' && !stateLine(row)
                  ? <ErrorText class="settings-row-warn" as="div" text={text()} />
                  : <div class="settings-row-v" role="status">{text()}</div>}
              </Show>
              <For each={knownBadLines(row)}>{line => <ErrorText class="settings-row-warn" as="div" text={line} />}</For>
            </div>
            <div class="settings-actions">
              <Show when={action()}>{a =>
                <button type="button" class="btn" disabled={isBusy(row.agent)}
                  title={a().kind === 'update' && row.updateCommand ? `Runs ${row.updateCommand}` : undefined}
                  onClick={() => void (a().kind === 'cancel' ? act(row.agent, () => api.cancelAgentCliUpdate(row.agent)) : update(row))}>
                  {isBusy(row.agent) ? 'Working…' : a().label}
                </button>}
              </Show>
              <Show when={row.state === 'waiting' && row.safeWhileRunning}>
                <button type="button" class="btn ghost" disabled={isBusy(row.agent)}
                  title="Running chats keep the version they started with"
                  onClick={() => void update(row, true)}>Update now</button>
              </Show>
              <Show when={row.canRestore && row.previousVersion && row.state === 'idle'}>
                <button type="button" class="btn ghost" disabled={isBusy(row.agent)}
                  onClick={() => void act(row.agent, () => api.restoreAgentCli(row.agent))}>Restore {row.previousVersion}</button>
              </Show>
              <Select size="sm" label={`${row.label} updates`} value={row.policy} disabled={!row.installed || isBusy(row.agent)}
                options={UPDATE_POLICIES.map(p => ({ value: p.value, label: p.label }))}
                onChange={value => void setPolicy(row, value as 'automatic' | 'ask' | 'off')} />
            </div>
          </div>
        );
  }
}
