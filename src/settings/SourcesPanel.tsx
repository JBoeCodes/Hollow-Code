// Settings, Sources: where your work lives (overhaul H4b with two-Macs T5;
// design `sources-and-import-2026-11-13` section 3; backend H4a,
// `commands/sources_api.rs`). Three role rows: Tasks and projects (T5's
// choice, Vault only or Vault plus Linear), Knowledge (the vault, plus
// read-only connector references) and Personal context (always the vault,
// no control). Changing where tasks live is a switch, never a Save: the dry
// run's counts show first, the switch needs a click and a confirm, and
// every switch can be rolled back here. Workspace overrides list below,
// each with its own switch. Words in `sourcesSettings.ts`; calls in
// `sourcesApi.ts`; both lazy with this panel, so neither reaches the App
// chunk.
import { For, Show, createMemo, createSignal, onMount, type JSX } from 'solid-js';
import { Button, ErrorText, Segmented } from '../components/ui';
import { Select } from '../components/Select';
import { SourceLabelChip } from '../components/SourceLabelChip';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { showConfirm } from '../store';
import { openSettingsFromOutside } from '../store/runtime';
import { workspaces } from '../store/workspaces';
import { errMsg, fmtTime } from '../utils';
import {
  applySourcesSwitch, planSourcesSwitch, rollBackSourcesSwitch, setSources, sourcesStatus, sourcesSwitches,
} from '../sourcesApi';
import type { SourcesMigrationPlan, SourcesMigrationStatus, SourcesOwnerKind, SourcesReference, SourcesRoleRow, SourcesStatus } from '../generated/ipc';
import {
  PERSONAL_LINE, bindingLine, planLines, switchBlocked, switchVerb, taskChoices, waitingLine, withReference, withoutReference,
  workspaceName,
} from '../sourcesSettings';

function RoleRow(props: { title: string; line: JSX.Element; children?: JSX.Element }) {
  return (
    <div class="settings-row sources-role" data-settings-control={props.title}>
      <div class="settings-row-label">
        <h4 class="settings-row-k">{props.title}</h4>
        <div class="settings-row-v">{props.line}</div>
        {props.children}
      </div>
    </div>
  );
}

export function SourcesPanel() {
  const [status, setStatus] = createSignal<SourcesStatus | undefined>();
  const [switches, setSwitches] = createSignal<SourcesMigrationStatus | undefined>();
  const [loadError, setLoadError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [note, setNote] = createSignal('');
  const [plan, setPlan] = createSignal<SourcesMigrationPlan | null>(null);
  const [pickWorkspace, setPickWorkspace] = createSignal('');
  const [connectors, setConnectors] = createSignal<{ id: string; name: string }[]>([]);
  const order = latestRequest();

  async function refetch() {
    const ticket = order.start();
    try {
      const [s, sw] = await Promise.all([sourcesStatus(), sourcesSwitches()]);
      if (!order.isLatest(ticket)) return;
      setStatus(s); setSwitches(sw); setLoadError('');
    } catch (e) {
      if (order.isLatest(ticket)) setLoadError(errMsg(e));
    }
  }
  onMount(() => {
    void refetch();
    api.listConnectors().then(v => setConnectors(v.connectors.map(c => ({ id: c.id, name: c.name || c.id }))), () => setConnectors([]));
  });

  async function act(run: () => Promise<unknown>, done?: string) {
    if (busy()) return;
    setBusy(true); setError(''); setNote('');
    try {
      await run();
      if (done) setNote(done);
      await refetch();
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  const row = (role: SourcesRoleRow['role']) => status()?.rows.find(r => r.role === role);
  const tasks = () => row('tasks');
  const providerName = () => tasks()?.binding.providerName ?? null;
  const blocked = () => switchBlocked(tasks(), status()?.unfinishedMigration ?? null);
  const references = () => status()?.references ?? [];
  const unusedConnectors = createMemo(() => connectors().filter(c => !references().some(r => r.reference.connector === c.id)));
  /** The tasks row shows the choice being dry-run until it is applied or cancelled. */
  const globalPlanTo = () => { const p = plan(); return p && p.target.workspace == null ? p.target.to : null; };
  // Roll back only the newest switch, and only once it applied (an unfinished one gets Finish).
  const latestApplied = () => { const newest = switches()?.switches[0]; return newest?.phase === 'applied' ? newest : null; };
  /** A workspace's name for a dry run or a switch line, its id when unknown. */
  const nameOf = (id: string) => status()?.workspaces.find(w => w.id === id)?.title || workspaces().find(w => w.id === id)?.title || id;

  /** A dry run first: nothing is written until the person confirms it. */
  async function planFor(to: SourcesOwnerKind, workspace: string | null) {
    await act(async () => { setPlan(await planSourcesSwitch({ workspace, to })); });
  }

  async function runSwitch(p: SourcesMigrationPlan) {
    const where = p.target.workspace ? `the workspace ${nameOf(p.target.workspace)}` : 'every workspace';
    const ok = await showConfirm({
      title: switchVerb(p.target.to, providerName()),
      summary: `Changes where tasks live for ${where}. Nothing is sent to the tracker.`,
      body: planLines(p).join('\n'),
      confirmLabel: 'Switch',
    });
    if (!ok) return;
    await act(async () => {
      const outcome = await applySourcesSwitch(p.target);
      setPlan(null);
      if (outcome.note) setNote(outcome.note);
    }, 'Switched. Roll back below if it was a mistake.');
  }

  async function rollBack(id: string) {
    const ok = await showConfirm({ title: 'Roll back this switch?', summary: 'Puts every file, the ledger and the settings back byte for byte.', confirmLabel: 'Roll back' });
    if (ok) await act(() => rollBackSourcesSwitch(id), 'Rolled back.');
  }

  const saveReferences = (list: SourcesReference[]) => act(() => setSources({ references: list }), 'Saved.');

  return (
    <div class="sources-panel">
      <Show when={loadError()}>{msg => <ErrorText as="p" class="settings-row-warn" text={`Couldn’t read where your work lives: ${msg()}`} />}</Show>
      <Show when={status()?.unfinishedMigration}>
        {id => (
          <div class="tracker-setup-note" role="alert">
            <p>A switch stopped part way ({id()}). Finishing it repeats only the steps left.</p>
            <Button size="sm" variant="secondary" disabled={busy()} onClick={() => {
              const rec = switches()?.switches.find(s => s.id === id());
              if (rec) void act(() => applySourcesSwitch(rec.target), 'Finished the switch.');
            }}>Finish the switch</Button>
          </div>
        )}
      </Show>

      <RoleRow title="Tasks and projects" line={<SourceLabelChip label={tasks()?.label} />}>
        <Show when={tasks()}>
          {t => (
            <div class="task-tracker-fields">
              <Segmented label="Where tasks live" items={taskChoices(providerName()).map(c => ({ value: c.value, label: c.label }))}
                value={globalPlanTo() ?? t().binding.owner}
                onChange={to => { setPlan(null); if (to !== t().binding.owner) void planFor(to, null); }} />
              <p class="settings-row-hint">{taskChoices(providerName()).find(c => c.value === t().binding.owner)?.hint}</p>
              <Show when={t().lastRead}>{at => <p class="settings-row-hint">Last read {fmtTime(at())}</p>}</Show>
              <Show when={waitingLine(t())}>{w => <p class="settings-row-hint">{w()}</p>}</Show>
              <For each={t().problems}>{p => <ErrorText as="p" class="settings-row-warn" text={p} />}</For>
              <Show when={t().binding.owner === 'tracker' || plan()?.target.to === 'tracker'}>
                <p class="settings-row-hint">The tracker’s key, team and sync interval are in Task tracker below.</p>
              </Show>
            </div>
          )}
        </Show>
      </RoleRow>

      <Show when={plan()}>
        {p => (
          <div class="sources-plan" role="region" aria-label="Switch dry run">
            <p class="settings-row-k">{switchVerb(p().target.to, providerName())}{p().target.workspace ? ` for ${nameOf(p().target.workspace!)}` : ''}: what would change</p>
            <ul class="sources-plan-lines"><For each={planLines(p())}>{line => <li>{line}</li>}</For></ul>
            <Show when={blocked()}>{why => <p class="settings-row-warn">{why()}</p>}</Show>
            <div class="settings-actions">
              <Button size="sm" disabled={busy() || !!p().refused || p().nothingToDo || !!blocked()} onClick={() => void runSwitch(p())}>{switchVerb(p().target.to, providerName())}</Button>
              <Button size="sm" variant="ghost" onClick={() => setPlan(null)}>Cancel</Button>
            </div>
          </div>
        )}
      </Show>

      <RoleRow title="Knowledge" line={<SourceLabelChip label={row('knowledge')?.label} />}>
        <div class="task-tracker-fields">
          <p class="settings-row-hint">Notes, docs and meeting records live in your vault. Agents may also read these connectors, read only.</p>
          <ul class="sources-references">
            <For each={references()}>
              {r => (
                <li>
                  <span>{r.reference.label || r.name || r.reference.connector}</span>
                  <SourceLabelChip label={r.label} />
                  <Show when={!r.inLibrary}><span class="settings-row-warn">Not in your connector library</span></Show>
                  <Button size="sm" variant="ghost" disabled={busy()} aria-label={`Remove ${r.name || r.reference.connector}`}
                    onClick={() => void saveReferences(withoutReference(references().map(x => x.reference), r.reference.connector))}>Remove</Button>
                </li>
              )}
            </For>
          </ul>
          <Show when={unusedConnectors().length}>
            <Select label="Add a read-only reference" placeholder="Add a read-only reference" size="sm" value=""
              options={unusedConnectors().map(c => ({ value: c.id, label: c.name }))}
              onChange={id => { if (id) void saveReferences(withReference(references().map(x => x.reference), id)); }} />
          </Show>
        </div>
      </RoleRow>

      <RoleRow title="Personal context" line={<span class="settings-state-chip" data-state="ok">{PERSONAL_LINE}</span>}>
        <div class="task-tracker-fields">
          <p class="settings-row-hint">Your profile, memory and learnings. Syncing them to your context repo is in Context.</p>
          <Button size="sm" variant="secondary" onClick={() => openSettingsFromOutside('layers')}>Open Context</Button>
        </div>
      </RoleRow>

      <RoleRow title="Workspace task sources" line={<span>{(status()?.workspaces.length ?? 0) ? `${status()!.workspaces.length} with their own setting` : 'Every workspace inherits'}</span>}>
        <div class="task-tracker-fields">
          <ul class="sources-workspaces">
            <For each={status()?.workspaces ?? []}>
              {w => (
                <li>
                  <span>{workspaceName(w)}</span>
                  <span class="settings-row-hint">{bindingLine(w.binding)}</span>
                  <SourceLabelChip label={w.label} />
                  <Button size="sm" variant="ghost" disabled={busy() || !!blocked()}
                    onClick={() => void planFor(w.binding.owner === 'tracker' ? 'vault' : 'tracker', w.id)}>
                    {w.binding.owner === 'tracker' ? 'Keep in vault only' : `Use ${w.binding.providerName || providerName() || 'Linear'}`}
                  </Button>
                </li>
              )}
            </For>
          </ul>
          <div class="settings-actions">
            <Select label="Workspace" placeholder="Choose a workspace" size="sm" value={pickWorkspace()}
              options={workspaces().filter(w => w.kind !== 'quick').map(w => ({ value: w.id, label: w.title || w.id }))}
              onChange={id => setPickWorkspace(id)} />
            <Button size="sm" variant="secondary" disabled={busy() || !pickWorkspace() || !!blocked()}
              onClick={() => void planFor(tasks()?.binding.owner === 'tracker' ? 'vault' : 'tracker', pickWorkspace())}>
              {tasks()?.binding.owner === 'tracker' ? 'Keep it in vault only' : `Track it in ${providerName() || 'Linear'}`}
            </Button>
          </div>
        </div>
      </RoleRow>

      <Show when={latestApplied()}>
        {rec => (
          <div class="settings-row sources-switches">
            <div class="settings-row-label">
              <div class="settings-row-k">Last switch</div>
              <div class="settings-row-v">{switchVerb(rec().target.to, providerName())}{rec().target.workspace ? ` for ${nameOf(rec().target.workspace!)}` : ''}, {fmtTime(rec().createdAt)}</div>
              <Button size="sm" variant="ghost" disabled={busy()} onClick={() => void rollBack(rec().id)}>Roll back</Button>
            </div>
          </div>
        )}
      </Show>

      <Show when={error()}>{msg => <ErrorText as="p" class="settings-row-warn" text={msg()} />}</Show>
      <Show when={note()}><p class="settings-row-hint" role="status">{note()}</p></Show>
    </div>
  );
}
