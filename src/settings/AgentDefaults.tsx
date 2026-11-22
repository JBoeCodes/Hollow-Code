// Settings rows extracted from pages/Settings.tsx (L4, X18): learning
// suggestions, canvases, the permission-mode row and the agent defaults form,
// which Settings S5b (2026-11-12) split into Models, Devices (the runner) and
// Advanced (Run limits, App builds), each piece with its own Save.
import { serializeAgentSettingsWrite } from './agentSettingsQueue';
import { ModelPicker } from '../components/ModelPicker';
import { Show, createResource, createSignal, createMemo, createEffect, For, untrack } from 'solid-js';
import { Segmented, ErrorText, PlainErrorText } from '../components/ui';
import { chatPermissionMode, newChatModeFor, newChatModeSource, setNewAgentPermissionModePersistent, saveAgentEffort, chatAgent, confirmPermissionModeChange, AGENT_LABELS, KNOWN_AGENTS, refreshAgentSettings, agentDescriptors, agentSettings, type ChatPermissionMode, type ChatAgent } from '../store';
import { errMsg } from '../utils';
import { CANVAS_FREQUENCIES, CANVAS_FREQUENCY_LABELS, canvasesForSave, settingCanvases, type CanvasFrequency } from '../canvasFrequency';
import type { Suggestions as LearningSuggestions } from '../generated/ipc';
import { api } from '../api';
import { permModeNote, permModeOptions, permModeUnavailable } from '../permModes';
import { agentModeDanger, agentModeDescriptions, agentModeLabels, agentModeOrder, hollowCardsLine, shownMode } from '../agentPermModes';
import { DEADLINE_MAX_MIN, RUNNER, WATCHDOG_DEFAULT_MIN, WATCHDOG_MAX_MIN, agentDeadline, agentSettingsPayload, agentWatchdog, appBuildsOnForSave, buildsOnForSave, deadlineMinutes, draftFromSettings, runsOnForSave, sameAgentSettings, savePayload, unattendedRunsOnForSave, watchdogMinutes, whenOutForSave, withAgentDeadline, withAgentWatchdog, withPiece, type AgentFormPiece } from '../agentSettingsPayload';
import type { AgentSettings } from '../types';
import { Select } from '../components/Select';
import { MoreFold, SettingsSubhead } from './settingsLayout';
import { RemoteRunnerRow } from './RemoteRunnerRow';
import { RouteDecisionsBlock } from './RunnerStatusBlock';
import { RunnerGithubRow } from './RunnerGithubRow';
import { CfoAllowRow, CfoEffortRow } from './CfoSettings';
import { preserveDuringRefetch, savedOrRead } from '../stableResource';
import { settingsRevision } from '../store/settingsRevision';
import { flashSaved, registerDirty } from './settingsDirty';

/// Settings, Personalization, How many learnings to suggest (fewer learnings, 10-11; renamed from
/// "Learning suggestions" in Settings PR 7, which `settingsResolve.ts` keeps as an alias): how
/// many learnings Hollow proposes from your chats and agents' runs. Saved at
/// once; the next turn reads it. Read again when an agent changes it
/// (`settingsRevision`, overnight H6), keeping the shown value meanwhile; a
/// save answers through `refetch(saved)` (`savedOrRead`) so a read in flight
/// can't put the older value back.
export function LearningSuggestionsSetting() {
  const [read, { refetch }] = createResource(() => ({ revision: settingsRevision() }), savedOrRead(() => api.getLearningSettings()));
  const settings = preserveDuringRefetch(read);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal('');
  const current = () => settings()?.suggestions ?? 'important';
  async function choose(mode: LearningSuggestions) {
    if (saving() || mode === current()) return;
    setSaving(true);
    setError('');
    try {
      refetch(await api.setLearningSuggestions(mode));
    } catch (e) {
      setError(`Could not save how many learnings to suggest: ${String(e)}`);
    } finally { setSaving(false); }
  }
  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">How many learnings to suggest</div>
          <div class="settings-row-v">{LEARNING_SUGGESTION_HINTS[current()]}</div>
        </div>
        <Segmented label="How many learnings to suggest" value={current()} onChange={value => void choose(value)}
          items={LEARNING_SUGGESTION_MODES.map(value => ({ value, label: LEARNING_SUGGESTION_LABELS[value] }))} />
      </div>
      <Show when={error()}>{msg => <ErrorText class="settings-row-warn" text={msg()}/>}</Show>
    </>
  );
}

export const LEARNING_SUGGESTION_MODES: readonly LearningSuggestions[] = ['off', 'important', 'all'];

export const LEARNING_SUGGESTION_LABELS: Record<LearningSuggestions, string> = { off: 'Off', important: 'Important only', all: 'All' };

export const LEARNING_SUGGESTION_HINTS: Record<LearningSuggestions, string> = {
  off: 'Hollow proposes no learnings. Teach Hollow still works.',
  important: 'One suggestion at most per exchange, only for lasting preferences or costly mistakes.',
  all: 'Up to three suggestions per exchange, whatever their impact.',
};

/// Settings, Chat, Canvases (Canvas C3 wave 2 C4): saved at once into the
/// agent settings, read by the next turn of every chat that has no choice of
/// its own (the model menu sets one per chat).
export function ChatCanvasesSetting() {
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal('');
  const current = () => settingCanvases(agentSettings());
  async function choose(choice: CanvasFrequency) {
    if (saving() || choice === current()) return;
    setSaving(true);
    setError('');
    try {
      const live = await api.agentSettings();
      await api.setAgentSettings(agentSettingsPayload({ ...live, canvases: canvasesForSave(choice) }));
      await refreshAgentSettings();
    } catch (e) {
      setError(`Could not save Canvases: ${String(e)}`);
    } finally { setSaving(false); }
  }
  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Canvases</div>
          <div class="settings-row-v">How often answers include a canvas.</div>
        </div>
        <Segmented label="Canvases" value={current()} onChange={value => void choose(value)}
          items={CANVAS_FREQUENCIES.map(value => ({ value, label: CANVAS_FREQUENCY_LABELS[value] }))} />
      </div>
      <Show when={error()}>{msg => <ErrorText class="settings-row-warn" text={msg()}/>}</Show>
    </>
  );
}

/// The agent defaults form (Settings S5b, 2026-11-12): one draft per piece,
/// each piece in its own section with its own Save. Every Save sends the
/// whole payload through the one builder (`savePayload`): this piece's fields
/// from its draft, every other field as the file holds it now, so a save in
/// one section never drops or reverts what another holds. Each piece has its
/// own unsaved-changes guard (`registerDirty`).
interface AgentFormWords { dirtyId: string; dirtyLabel: string; save: string; saved: string; failed: string }

const AGENT_FORM_WORDS: Record<AgentFormPiece, AgentFormWords> = {
  // Models keeps the form's old id, button and words.
  models: { dirtyId: 'agent-defaults', dirtyLabel: 'Agent defaults', save: 'Save agent defaults', saved: 'Agent defaults saved. New runs use these settings; existing conversations keep their agent.', failed: 'Could not save agent defaults' },
  runner: { dirtyId: 'agent-defaults-runner', dirtyLabel: 'Runner settings', save: 'Save runner settings', saved: 'Runner settings saved. Turns that start now use them.', failed: 'Could not save the runner settings' },
  limits: { dirtyId: 'agent-defaults-limits', dirtyLabel: 'Run limits', save: 'Save run limits', saved: 'Run limits saved. New runs use them.', failed: 'Could not save the run limits' },
  appBuilds: { dirtyId: 'agent-defaults-app-builds', dirtyLabel: 'App builds', save: 'Save app builds', saved: 'App builds saved. The next Build app uses it.', failed: 'Could not save App builds' },
};

function createAgentForm(piece: AgentFormPiece) {
  const words = AGENT_FORM_WORDS[piece];
  const [loadError, setLoadError] = createSignal('');
  // Read again whenever a setting changes elsewhere (an effort from the
  // composer, an agent's `change_setting`), so the draft rebases (P0-19).
  const [read, { refetch }] = createResource(() => ({ revision: settingsRevision() }), async () => {
    setLoadError('');
    try { return await api.agentSettings(); }
    catch (error) { setLoadError(errMsg(error)); return null; }
  });
  // A refetch keeps the shown value instead of suspending the whole Settings page (review).
  const settings = preserveDuringRefetch(read);
  const [draft, setDraft] = createSignal<AgentSettings>({ defaultAgent: null, models: {} });
  const [saving, setSaving] = createSignal(false);
  const [message, setMessage] = createSignal('');
  const [failed, setFailed] = createSignal(false);
  /** Whether `draft` changes this piece's fields of `value`. */
  const changes = (value: AgentSettings, d: AgentSettings) => !sameAgentSettings(value, withPiece(value, d, piece));
  /** The settings the draft was last taken from. */
  let base: AgentSettings | null = null;
  createEffect(() => {
    const value = settings();
    if (!value) return;
    // Every field (A4): one left out opens the pane dirty. Unsaved edits
    // survive a fresh read; an untouched draft follows it.
    const edited = base !== null && changes(base, untrack(draft));
    base = value;
    if (!edited) setDraft(draftFromSettings(value));
  });
  // Compared as saved payloads (Paperclip PR 14a), so edit order and an
  // absent-versus-empty `efforts` never read as a change.
  const dirty = createMemo(() => {
    const value = settings();
    return !!value && changes(value, draft());
  });
  // Unsaved form changes: a section switch asks first (P1-21).
  registerDirty(words.dirtyId, () => dirty() && !saving(), words.dirtyLabel);
  /** Change the draft (and drop the last save's answer). */
  function edit(change: (previous: AgentSettings) => AgentSettings) {
    setMessage('');
    setDraft(change);
  }
  async function save() {
    if (saving() || !settings()) return;
    setSaving(true);
    setMessage('');
    setFailed(false);
    try {
      // One payload builder for every field (cross-m34 M1): this piece's
      // fields from the draft, the rest from the file read just now (A4).
      // The read, merge and write run on one queue shared by every piece, so two pieces saving
      // at once cannot both read before either writes and revert each other.
      await serializeAgentSettingsWrite(async () => {
        const live = await api.agentSettings();
        await api.setAgentSettings(savePayload(withPiece(draftFromSettings(live), draft(), piece), live));
      });
      await refreshAgentSettings();
      // The saved draft is the new base: take the fresh read as it is.
      base = null;
      await refetch();
      setMessage(words.saved);
    } catch (error) {
      setFailed(true);
      setMessage(`${words.failed}: ${errMsg(error)}`);
    } finally { setSaving(false); }
  }
  const disabled = () => !settings() || saving();
  /** Its load error with Retry, above the piece's rows. */
  const Head = () => (
    <Show when={loadError()}>
      <p class="settings-row-warn" role="alert">Could not load agent defaults: <PlainErrorText text={loadError()} /></p>
      <button class="btn" onClick={() => refetch()}>Retry agent settings</button>
    </Show>
  );
  /** Its Save, the Unsaved line and the save's answer (P1-21). */
  const Foot = () => (
    <>
      <div class="settings-actions">
        <button class="btn primary" onClick={save} disabled={!dirty() || saving()}>{saving() ? 'Saving…' : words.save}</button>
      </div>
      <Show when={dirty() && !saving()}><p class="settings-note">Unsaved changes. {words.save} keeps them; switching Settings sections asks first.</p></Show>
      <Show when={message()}><p class={failed() ? 'settings-row-warn' : 'settings-note'} role={failed() ? 'alert' : 'status'}><PlainErrorText text={message()} plain={failed()} /></p></Show>
    </>
  );
  return { settings, draft, edit, disabled, Head, Foot };
}

/// The runner's name, read once per piece (a signal, not a resource, so the
/// read never suspends the page); undefined until read.
function createRunnerName() {
  const [runner, setRunner] = createSignal<string | null | undefined>(undefined);
  void api.remoteRunner().then(setRunner, () => undefined);
  return [runner, setRunner] as const;
}

/** Builds and tests as its Select shows it: This Mac (the default since M7)
 *  for absent or a saved `this-mac`. */
const buildsOnShown = (value: string | null | undefined) => {
  const v = buildsOnForSave(value);
  return v === 'this-mac' ? '' : v ?? '';
};

const remoteOption = (runner: string | null | undefined) => ({ value: RUNNER, label: 'Remote runner', detail: runner ? runner : 'No runner set up' });

/// Settings, Models: Default agent, When an agent runs out, each agent's
/// model (installed agents first, the rest behind Not installed) with Save
/// agent defaults, then Effort and Models the CFO may use, which save as you
/// change them (plan 3.2 row 3).
export function AgentDefaults(props: { installed: string[] }) {
  const form = createAgentForm('models');
  const { draft, edit, disabled } = form;
  /** The effort levels an agent accepts (none: no effort row). */
  const effortLevels = (agent: string) =>
    agentDescriptors().find(descriptor => descriptor.id === agent)?.capabilities.effortLevels ?? [];
  function updateModel(agent: string, value: string) {
    edit(previous => {
      const models = { ...previous.models };
      if (value) models[agent] = value;
      else delete models[agent];
      return { ...previous, models };
    });
  }
  /** Effort saves on change, like the composer's Effort list and through
   *  the same setter (P0-19); it isn't part of the Save form above. */
  // Its answer shows under the effort rows, outside the Save form (P1-21).
  const [effortMessage, setEffortMessage] = createSignal('');
  const [effortFailed, setEffortFailed] = createSignal(false);
  async function updateEffort(agent: string, value: string) {
    setEffortMessage('');
    setEffortFailed(false);
    try {
      await saveAgentEffort(agent, value);
      const name = AGENT_LABELS[agent as keyof typeof AGENT_LABELS] ?? agent;
      setEffortMessage(`${name} effort saved: ${value || 'runtime default'}. New turns use it.`);
      flashSaved(`${name} effort`);
    } catch (error) {
      setEffortFailed(true);
      setEffortMessage(`Could not save the effort: ${errMsg(error)}`);
    }
  }
  const installed = () => KNOWN_AGENTS.filter(agent => props.installed.includes(agent));
  const others = () => KNOWN_AGENTS.filter(agent => !props.installed.includes(agent));
  const ModelRow = (row: { agent: (typeof KNOWN_AGENTS)[number] }) => (
    <div class="settings-row">
      <div class="settings-row-label">
        <label class="settings-row-k" for={`default-model-${row.agent}`}>{AGENT_LABELS[row.agent]} model</label>
      </div>
      <ModelPicker id={`default-model-${row.agent}`} label={`${AGENT_LABELS[row.agent]} model`}
        agent={row.agent} value={draft().models[row.agent] ?? ''} onChange={value => updateModel(row.agent, value)}
        disabled={disabled()} />
    </div>
  );
  return (
    <div class="agent-defaults">
      <form.Head />
      {/* What Save agent defaults keeps, framed with its Save and an
          Unsaved line (P1-21); Effort, below, saves as you change it. */}
      <div class="agent-defaults-form" role="group" aria-label="Saved with Save agent defaults">
      <div class="settings-row">
        <div class="settings-row-label">
          {/* S5b: no help line; the Automatic option says what it does. */}
          <label for="default-agent" class="settings-row-k">Default agent</label>
        </div>
        <Select id="default-agent" label="Default agent" value={draft().defaultAgent ?? ''} disabled={disabled()} searchable={false}
          onChange={value => edit(previous => ({ ...previous, defaultAgent: value || null }))}
          options={[{ value: '', label: 'Automatic · installed agent' }, ...KNOWN_AGENTS.map(agent => ({ value: agent, label: AGENT_LABELS[agent], detail: props.installed.includes(agent) ? undefined : 'Not installed' }))]} />
      </div>
      {/* Repo ideas 2026-10-27 #8: Wait is the default (absent). */}
      <div class="settings-row">
        <div class="settings-row-label">
          <label for="when-agent-out" class="settings-row-k">When an agent runs out</label>
          <div class="settings-row-v">Wait, or continue on another agent if the turn changed nothing yet.</div>
        </div>
        <Select id="when-agent-out" label="When an agent runs out" value={whenOutForSave(draft().whenOut) ?? ''} disabled={disabled()} searchable={false}
          onChange={value => edit(previous => ({ ...previous, whenOut: value || undefined }))}
          options={[{ value: '', label: 'Wait for it' }, ...KNOWN_AGENTS.map(agent => ({ value: agent, label: `Continue on ${AGENT_LABELS[agent]}`, detail: props.installed.includes(agent) ? undefined : 'Not installed' }))]} />
      </div>
      <SettingsSubhead title="Models" />
      <For each={installed()}>{agent => <ModelRow agent={agent} />}</For>
      {/* Plan 4.2 row 35: agents that aren't installed wait behind one closed fold. */}
      <Show when={others().length > 0}>
        <details class="settings-fold settings-row-more agent-defaults-others">
          <summary>Not installed: {others().map(agent => AGENT_LABELS[agent]).join(', ')}</summary>
          <For each={others()}>{agent => <ModelRow agent={agent} />}</For>
        </details>
      </Show>
      <form.Foot />
      </div>
      {/* COO rebuild L7: Automatic (the CFO) or Fixed; Fixed shows the per-agent levels. */}
      <CfoEffortRow>
      <For each={KNOWN_AGENTS.filter(agent => effortLevels(agent).length > 0)}>{agent => (
        <div class="settings-row">
          <div class="settings-row-label">
            <label class="settings-row-k" for={`default-effort-${agent}`}>{AGENT_LABELS[agent]} effort</label>
            <div class="settings-row-v">Used by chat, automations and teammates. A model without this level gets the nearest.</div>
          </div>
          <Select id={`default-effort-${agent}`} label={`${AGENT_LABELS[agent]} effort`} value={form.settings()?.efforts?.[agent] ?? ''} disabled={disabled()} searchable={false}
            onChange={value => void updateEffort(agent, value)}
            options={[{ value: '', label: 'Runtime default' }, ...effortLevels(agent).map(level => ({ value: level, label: level }))]} />
        </div>
      )}</For>
      </CfoEffortRow>
      <CfoAllowRow installed={props.installed} />
      <Show when={effortMessage()}><p class={effortFailed() ? 'settings-row-warn' : 'settings-note'} role={effortFailed() ? 'alert' : 'status'}><PlainErrorText text={effortMessage()} plain={effortFailed()} /></p></Show>
    </div>
  );
}

/// Settings, Devices, the runner (plan 3.2 row 9): Remote runner (Set up,
/// Set up again, Turn off), then, once one is set up, Runs on, Builds and
/// tests, Overnight and worker chats and Runner GitHub access, with Save
/// runner settings. App builds is in Advanced, App builds and web backend.
export function RunnerSettings() {
  const form = createAgentForm('runner');
  const { draft, edit, disabled } = form;
  // Remote runner's Set up and Turn off change it.
  const [runner, setRunner] = createRunnerName();
  return (
    <div class="agent-defaults agent-defaults-runner">
      <form.Head />
      {/* Remote runners 2 (2026-11-03): Off until the person sets one up. */}
      <RemoteRunnerRow runner={runner()} onChange={setRunner} />
      {/* Overnight 11-07 (lane D3): with no runner set up there is nothing to route, so the rows that
          choose what runs there show once one is set; Remote runner's Set up, just above, is their
          visible way in, and Settings' search lands on it for any of them (`near`). */}
      <Show when={runner()}>
      <div class="agent-defaults-form" role="group" aria-label="Saved with Save runner settings">
      {/* The remote runner (2026-11-01): this Mac is the default (absent). */}
      <div class="settings-row">
        <div class="settings-row-label">
          <label for="runs-on" class="settings-row-k">Runs on</label>
          <div class="settings-row-v">Where a build workspace's agent turns run. Runner: {runner()}.</div>
          <MoreFold>
            <div class="settings-row-v">A self-hosted model's turns stay on this Mac. Another workspace's chats run on the runner only when that workspace chooses it in a new conversation; the COO and Chats always run here. On the runner only the agent runs there, in its own copy of the worktree or workspace folder, using one of your added Claude accounts; the conversation, History, tools and Stop stay on this Mac, and its work comes back when the turn ends (a file changed on both sides is merged or kept twice).</div>
          </MoreFold>
        </div>
        <Select id="runs-on" label="Runs on" value={runsOnForSave(draft().runsOn) ?? ''} disabled={disabled()} searchable={false}
          onChange={value => edit(previous => ({ ...previous, runsOn: value || undefined }))}
          options={[{ value: '', label: 'This Mac' }, remoteOption(runner())]} />
      </div>
      {/* Smart routing 3 (2026-11-03): This Mac is the default (absent) since M7; Auto is a saved choice. */}
      <div class="settings-row">
        <div class="settings-row-label">
          <label for="builds-on" class="settings-row-k">Builds and tests</label>
          <div class="settings-row-v">Where hollow-slot runs builds and tests. This Mac by default.</div>
          <MoreFold>
            <div class="settings-row-v">Auto decides each one: gates, Rust builds, whole test suites, typechecks, lints and builds go to the runner; one test file, anything that needs this Mac (its database, gcloud, GitHub, the keychain, files outside the checkout) and anything that writes into the checkout stay here. Remote runner sends whatever can go there; This Mac keeps every command here unless it asks for the runner itself (hollow-slot --remote). App install builds follow App builds, in Advanced. Turns that start after you save use it.</div>
          </MoreFold>
          <RouteDecisionsBlock />
        </div>
        {/* M7 (PER-21): This Mac is the default (absent; a saved this-mac shows as it). */}
        <Select id="builds-on" label="Builds and tests" value={buildsOnShown(draft().buildsOn)} disabled={disabled()} searchable={false}
          onChange={value => edit(previous => ({ ...previous, buildsOn: value || undefined }))}
          options={[{ value: '', label: 'This Mac' }, { value: 'auto', label: 'Auto', detail: 'Decides for each command' }, remoteOption(runner())]} />
      </div>
      {/* Overnight N6 (2026-11-02): Follow Runs on is the default (absent) since M7. */}
      <div class="settings-row">
        <div class="settings-row-label">
          <label for="unattended-runs-on" class="settings-row-k">Overnight and worker chats</label>
          <div class="settings-row-v">Where night-queue items and unattended chats run in a build workspace.</div>
          <MoreFold>
            <div class="settings-row-v">With Runner when it answers, the remote runner takes them when it can; when it doesn't answer, isn't taking work, or can't run that agent (no added Claude account, Codex without an API key), they run on this Mac and the chat says why, even when Runs on is Remote runner. Follow Runs on keeps them with Runs on. A chat on a self-hosted model runs on this Mac either way, and its first line says why.</div>
          </MoreFold>
        </div>
        <Select id="unattended-runs-on" label="Overnight and worker chats" value={unattendedRunsOnForSave(draft().unattendedRunsOn) === RUNNER ? RUNNER : ''} disabled={disabled()} searchable={false}
          onChange={value => edit(previous => ({ ...previous, unattendedRunsOn: value || undefined }))}
          options={[{ value: '', label: 'Follow Runs on' }, { value: RUNNER, label: 'Runner when it answers', detail: runner() ? runner()! : 'No runner set up' }]} />
      </div>
      <form.Foot />
      </div>
      {/* Remote runners (2026-11-03): the runner turns' GitHub credential, saved on its own. */}
      <RunnerGithubRow />
      </Show>
    </div>
  );
}

/// Settings, Advanced, Run limits (plan 3.3, 4.2 rows 36 to 38): each
/// agent's no-output watchdog and run deadline, with Save run limits.
export function RunLimits() {
  const form = createAgentForm('limits');
  const { draft, edit, disabled } = form;
  /** Whether `agent`'s watchdog is on in the draft. */
  const watchdogOn = (agent: string) => agentWatchdog(draft(), agent) !== null;
  const updateWatchdog = (agent: string, minutes: number | null) => edit(previous => withAgentWatchdog(previous, agent, minutes));
  const updateDeadline = (agent: string, minutes: number | null) => edit(previous => withAgentDeadline(previous, agent, minutes));
  return (
    <div class="agent-defaults agent-defaults-limits">
      <form.Head />
      <div class="agent-defaults-form" role="group" aria-label="Saved with Save run limits">
      <SettingsSubhead title="No-output watchdog" hint="A run that prints nothing for this long is stopped as stalled." />
      <For each={KNOWN_AGENTS}>{agent => (
        <div class="settings-row">
          <div class="settings-row-label">
            <label class="settings-row-k" for={`watchdog-minutes-${agent}`}>Stop a silent {AGENT_LABELS[agent]} run after</label>
            <div class="settings-row-v">Minutes, 1 to 1440 (default 30). Off lets a quiet run go on until you stop it.</div>
          </div>
          <div class="settings-watchdog">
            <input
              id={`watchdog-minutes-${agent}`}
              type="number"
              min="1"
              max={String(WATCHDOG_MAX_MIN)}
              step="1"
              class="settings-input"
              value={agentWatchdog(draft(), agent) ?? WATCHDOG_DEFAULT_MIN}
              disabled={disabled() || !watchdogOn(agent)}
              onChange={event => updateWatchdog(agent, watchdogMinutes(Number(event.currentTarget.value)))}
            />
            <label class="settings-watchdog-off">
              <input
                type="checkbox"
                aria-label={`${AGENT_LABELS[agent]} watchdog off`}
                checked={!watchdogOn(agent)}
                disabled={disabled()}
                onChange={event => updateWatchdog(agent, event.currentTarget.checked ? null : WATCHDOG_DEFAULT_MIN)}
              />
              Off
            </label>
          </div>
        </div>
      )}</For>
      <SettingsSubhead title="Run deadline" hint="Optional. A run still going after this long is stopped as timed out." />
      <For each={KNOWN_AGENTS}>{agent => (
        <div class="settings-row">
          <div class="settings-row-label">
            <label class="settings-row-k" for={`deadline-minutes-${agent}`}>End a {AGENT_LABELS[agent]} run after</label>
            <div class="settings-row-v">Minutes, 1 to 1440. Leave empty for no deadline.</div>
          </div>
          <input
            id={`deadline-minutes-${agent}`}
            type="number"
            min="1"
            max={String(DEADLINE_MAX_MIN)}
            step="1"
            class="settings-input"
            placeholder="No deadline"
            value={agentDeadline(draft(), agent) ?? ''}
            disabled={disabled()}
            onChange={event => updateDeadline(agent, event.currentTarget.value.trim() === '' ? null : deadlineMinutes(Number(event.currentTarget.value)))}
          />
        </div>
      )}</For>
      <form.Foot />
      </div>
    </div>
  );
}

/// Settings, Advanced, App builds and web backend (plan 3.3, 4.2 row 32):
/// where Build app compiles Hollow, with Save app builds. Shown with or
/// without a runner: with none, Remote runner says so and a build stays here.
export function AppBuildsRow() {
  const form = createAgentForm('appBuilds');
  const { draft, edit, disabled } = form;
  const [runner] = createRunnerName();
  return (
    <div class="agent-defaults agent-defaults-app-builds">
      <form.Head />
      {/* Smart routing 7 (2026-11-03): this Mac is the default (absent). */}
      <div class="agent-defaults-form" role="group" aria-label="Saved with Save app builds">
      <div class="settings-row">
        <div class="settings-row-label">
          <label for="app-builds-on" class="settings-row-k">App builds</label>
          <div class="settings-row-v">Where Build app compiles Hollow. If the runner can’t, it builds here.</div>
          <MoreFold>
            <div class="settings-row-v">Remote runner compiles it there, signed ad hoc because the runner has no signing key; this Mac then signs it again with its own identity, and the build waits in the Update button as usual. What the runner builds is signed with this Mac's identity and gets Hollow's permissions, so use it only with a runner you control; it may also link against an older macOS SDK than this Mac's, so try one build there by hand before relying on it. The runner is set up in Devices.</div>
          </MoreFold>
        </div>
        <Select id="app-builds-on" label="App builds" value={appBuildsOnForSave(draft().appBuildsOn) ?? ''} disabled={disabled()} searchable={false}
          onChange={value => edit(previous => ({ ...previous, appBuildsOn: value || undefined }))}
          options={[{ value: '', label: 'This Mac' }, remoteOption(runner())]} />
      </div>
      <form.Foot />
      </div>
    </div>
  );
}

/// Permission mode for new chats (P0-19: Settings owns defaults, the chat's
/// own menu owns that chat). Since 2026-11-12 one per agent, each in that
/// agent's own words (`agentPermModes.ts`): Auto for Claude and Codex and
/// Accept edits for Grok and Antigravity until one is saved
/// (`newChatModeFor`; the single setting from before is retired). The row
/// named "Permission mode for new chats" only heads the four agent rows, each
/// the one place its agent's default is set. A danger mode asks first
/// (`confirmPermissionModeChange`), like the chat's menus and
/// `/permissions`. Changing one never touches the chat on screen. A mode the
/// agent can't run (Antigravity has no Auto) stays listed but disabled with
/// the backend's reason.
const NEW_MODE_SOURCE = { own: '', default: ' (the default)' } as const;
export function PermissionModeRow() {
  const [said, setSaid] = createSignal('');
  const capsOf = (agent: ChatAgent) => agentDescriptors().find(a => a.id === agent)?.capabilities;
  async function onChange(agent: ChatAgent, value: string) {
    const next = value as ChatPermissionMode;
    if (next === newChatModeFor(agent) || permModeUnavailable(capsOf(agent), next)) return;
    // The control stays on the current mode while the confirmation is open;
    // it changes only once the change is confirmed.
    const ok = await confirmPermissionModeChange(next, agent);
    if (!ok) return;
    setNewAgentPermissionModePersistent(agent, next);
    const labels = agentModeLabels(agent);
    setSaid(`New ${AGENT_LABELS[agent]} chats start in ${labels[next]}. This chat stays in ${agentModeLabels(chatAgent())[shownMode(chatAgent(), chatPermissionMode())]}; change it from its own menu.`);
  }
  return (
    <>
      <div class="settings-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Permission mode for new chats</div>
          <div class="settings-row-v">Each agent starts new chats in its own mode, named as it names them.</div>
          {/* Settings words budget (2026-11-15): a risky row's longer help sits behind its own More. */}
          <MoreFold><div class="settings-row-v">{hollowCardsLine('claude')}</div></MoreFold>
          <Show when={said()}><div class="settings-note" role="status">{said()}</div></Show>
        </div>
      </div>
      <For each={KNOWN_AGENTS}>
        {agent => {
          const cur = () => newChatModeFor(agent);
          const labels = () => agentModeLabels(agent);
          const note = () => `${permModeNote(labels()[cur()], agentModeDescriptions(agent)[cur()], capsOf(agent), cur())}${NEW_MODE_SOURCE[newChatModeSource(agent)]}`;
          // The menu beside it names the mode and lists each one's line, so a safe, available
          // mode's line sits behind More; a mode that skips checks or can't run says so in view.
          const inView = () => !!agentModeDanger(agent)[cur()] || !!permModeUnavailable(capsOf(agent), cur());
          return (
            <div class="settings-row">
              <div class="settings-row-label">
                <div class="settings-row-k">New {AGENT_LABELS[agent]} chats</div>
                <Show when={inView()} fallback={<MoreFold><div class="settings-row-v">{note()}</div></MoreFold>}>
                  <div class="settings-row-v">{note()}</div>
                </Show>
              </div>
              <Select label={`Permission mode for new ${AGENT_LABELS[agent]} chats`} class={`settings-select ${agentModeDanger(agent)[cur()] ? 'danger' : ''}`} value={cur()} onChange={v => void onChange(agent, v)} searchable={false}
                options={permModeOptions(agentModeOrder(agent), labels(), agentModeDescriptions(agent), capsOf(agent))} />
            </div>
          );
        }}
      </For>
    </>
  );
}
