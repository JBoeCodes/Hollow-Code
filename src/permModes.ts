// Which permission modes an agent can run (merged plan slice 2). The backend
// is the source: `agent::capabilities` sends `unavailableModes` (mode →
// reason, from `agent::unavailable_mode`), and a run in such a mode fails
// closed with the same reason. Antigravity has no Auto: in the 2026-10-02
// probe matrix its sandbox held only shell commands, and its file tools wrote
// outside the working folder. Pure helpers, no store, so vitest can drive them.
import type { AgentDescriptor } from './types';

type Capabilities = AgentDescriptor['capabilities'] | undefined;

// `default` since the per-agent menus (2026-11-12): Claude's and Grok's
// Default, Codex's Ask for approval.
type PermissionMode = 'acceptEdits' | 'auto' | 'default' | 'plan' | 'bypassPermissions';
const knownMode = (value: unknown): value is PermissionMode => typeof value === 'string' && ['acceptEdits', 'auto', 'default', 'plan', 'bypassPermissions'].includes(value);
const CHOICES_KEY = 'hollow.chat.modes.v1';
const CHOICE_LIMIT = 500;
const choices = new Map<string, PermissionMode>();
let loaded = false;
let migrated = false;
// A quota fallback cannot retain every explicit choice. Missing chats must
// then start in Plan, never resurrect a broader recorded execution mode.
let conservativeFallback = false;
// The record on disk lacks choices this window holds: a write failed, so the
// disk holds the Plan fallback alone (or the record from before, if even that
// failed), until a write saves the whole record again. The phone chat API's
// copy follows the disk, which is what this window shows once reopened.
let unsaved = false;
// The record on disk changed wholesale since the phone chat API's copy last
// heard all of it, so the next report replaces that copy.
let wholesale = false;

function loadChoices() {
  if (loaded) return;
  loaded = true;
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(CHOICES_KEY) ?? '[]');
    const record = stored && typeof stored === 'object' && !Array.isArray(stored) ? stored as Record<string, unknown> : null;
    conservativeFallback = stored === 'plan' || record?.fallback === 'plan';
    const rows = record ? record.choices : stored;
    if (Array.isArray(rows)) for (const row of rows.slice(-CHOICE_LIMIT)) {
      if (Array.isArray(row) && typeof row[0] === 'string' && row[0].length <= 256 && knownMode(row[1])) choices.set(row[0], row[1]);
    }
  } catch { /* unavailable or malformed storage: use recorded execution mode */ }
}

const serializedChoices = () => JSON.stringify(conservativeFallback ? { fallback: 'plan', choices: [...choices] } : [...choices]);

/** At most 500 recent choices, in one bounded record. Memory is authoritative
 * for this process, including when a newer choice could not be written. */
export function rememberConversationMode(key: string | null, mode: PermissionMode) {
  if (!key || key.length > 256 || !knownMode(mode)) return true;
  loadChoices();
  choices.delete(key); choices.set(key, mode);
  while (choices.size > CHOICE_LIMIT) choices.delete(choices.keys().next().value!);
  try {
    localStorage.setItem(CHOICES_KEY, serializedChoices());
    // Saved whole again after a failed write: the disk changed wholesale.
    if (unsaved) { unsaved = false; wholesale = true; }
    return true;
  } catch {
    conservativeFallback = true;
    unsaved = true; wholesale = true;
    // A tiny durable sentinel protects ALL dropped choices, not just the
    // latest. Keep memory intact; report non-durability even if this saves.
    try {
      localStorage.removeItem(CHOICES_KEY);
      localStorage.setItem(CHOICES_KEY, JSON.stringify('plan'));
    } catch { /* completely unavailable storage: caller warns */ }
    return false;
  }
}

function savedChoice(key: string): PermissionMode | undefined {
  loadChoices();
  const current = choices.get(key);
  if (current) return current;
  if (conservativeFallback) return 'plan';
  // The retired drawer had its own choices. Consume them only for this chat.
  try {
    const legacy = localStorage.getItem(`hollow.mainDrawer.mode.${key}`);
    if (knownMode(legacy)) {
      rememberConversationMode(key, legacy);
      // Delete only after the bounded replacement was durably written.
      if (localStorage.getItem(CHOICES_KEY) === serializedChoices()) localStorage.removeItem(`hollow.mainDrawer.mode.${key}`);
      return legacy;
    }
  } catch { /* no legacy choice */ }
  return undefined;
}

/** Every chat's choice as this window keeps it on disk, oldest first, and
 * whether a chat it keeps none for starts in Plan here: what the phone chat
 * API keeps so a phone's send runs in the mode the window shows for that chat
 * (`phone_access_chat_modes`). After a failed write the disk holds the Plan
 * fallback alone, so this is no choice and Plan for every chat, which is what
 * the window shows once reopened (a phone's send never runs broader than
 * that), until a write saves the whole record again. The retired drawer's
 * choices not moved over yet count, as `savedChoice` would read them. */
export function conversationModeChoices(): { choices: [string, string][]; plan: boolean } {
  loadChoices();
  if (unsaved) return { choices: [], plan: true };
  const all: [string, string][] = [];
  if (!conservativeFallback) {
    try {
      const prefix = 'hollow.mainDrawer.mode.';
      for (let i = 0; i < localStorage.length; i++) {
        const name = localStorage.key(i);
        const key = name?.startsWith(prefix) ? name.slice(prefix.length) : null;
        const mode = name ? localStorage.getItem(name) : null;
        if (key && key.length <= 256 && !choices.has(key) && knownMode(mode)) all.push([key, mode]);
      }
    } catch { /* storage that can't be listed: the record alone */ }
  }
  all.push(...choices);
  return { choices: all.slice(-CHOICE_LIMIT), plan: conservativeFallback };
}

/** A report for the phone chat API's copy of this window's choices. */
export interface ModeReport { choices: [string, string][]; replace: boolean; plan: boolean }

/** The whole record, replacing the phone chat API's copy (Companion devices
 * opening, or Hollow asking until this window answers after a launch). */
export function conversationModeRecord(): ModeReport {
  wholesale = false;
  return { ...conversationModeChoices(), replace: true };
}

/** What the phone chat API's copy needs after the person chose `mode` for
 * chat `key` (and it was remembered): that one choice; or the whole record
 * when the disk changed wholesale since the copy last heard all of it (a
 * write failed and left the Plan fallback alone there, or the whole record
 * was saved again after one) or still lacks choices (second review,
 * 2026-11-04: the copy kept up to 500 choices the window had lost). */
export function conversationModeReport(key: string | null, mode: string): ModeReport | null {
  if (!key || key.length > 256 || !knownMode(mode)) return null;
  loadChoices();
  if (wholesale || unsaved) return conversationModeRecord();
  return { choices: [[key, mode]], replace: false, plan: conservativeFallback };
}

/** Migrate only the last visible pre-upgrade choice, never a global default. */
export function preserveCurrentModeOnUpgrade(key: string | null, mode: unknown) {
  if (migrated) return;
  migrated = true;
  try {
    if (localStorage.getItem('hollow.chat.mode.migrated') === '1') return;
    if (key && knownMode(mode) && !rememberConversationMode(key, savedChoice(key) ?? mode)) return;
    localStorage.setItem('hollow.chat.mode.migrated', '1');
  } catch { /* memory retains this launch's migration */ }
}

/** What of a permission mode a native session is bound to: the backend's
 * `chat_resume::mode_class` (HB01). `plan` reads only; every other mode is
 * `act`, because Claude, Codex and Antigravity take the mode on every
 * process, resumes included. Grok keeps the sandbox profile its session
 * started with, so for Grok the class is that profile. Two modes of one
 * class share a session: a safer mode never resumes broader permissions. */
export function modeClass(agent: string | null | undefined, mode: string): string {
  if (mode === 'plan') return 'plan';
  if (agent !== 'grok') return 'act';
  // `agent::grok_sandbox`: `default` (an agent's child task can ask for it) reads only.
  if (mode === 'default') return 'grok:read-only';
  return mode === 'auto' || mode === 'bypassPermissions' ? 'grok:off' : 'grok:workspace';
}

/** Resume a native session only when its recorded mode is in this chat's
 * selected mode's class ([`modeClass`]): an Edits-only check-in and the
 * person's Bypass turn share one (2026-11-04: each turn after a Main
 * check-in used to start a fresh session, and the check-in's question card
 * couldn't be sent). Unknown history starts conservatively with retained
 * context. */
export function conversationPermission(key: string, recorded?: string | null, agent?: string | null): { mode: PermissionMode; resumeNative: boolean } {
  const mode = savedChoice(key) ?? (knownMode(recorded) ? recorded : 'acceptEdits');
  return { mode, resumeNative: knownMode(recorded) && modeClass(agent, recorded) === modeClass(agent, mode) };
}

/** The reason `mode` is unavailable for the agent with these capabilities,
 *  or null when it can run. Unknown capabilities (descriptors not loaded yet)
 *  disable nothing: the backend still refuses the run. */
export function permModeUnavailable(caps: Capabilities, mode: string): string | null {
  const reason = caps?.unavailableModes?.[mode];
  return typeof reason === 'string' && reason.trim() ? reason.trim() : null;
}

export interface PermModeOption<M extends string> {
  value: M;
  label: string;
  detail: string;
  disabled?: boolean;
}

/** Picker options in `order`: an unavailable mode stays listed, disabled,
 *  labelled "(unavailable)" and explained by its reason. */
export function permModeOptions<M extends string>(
  order: readonly M[],
  labels: Record<M, string>,
  descriptions: Record<M, string>,
  caps: Capabilities,
): PermModeOption<M>[] {
  return order.map((mode) => {
    const reason = permModeUnavailable(caps, mode);
    return reason
      ? { value: mode, label: `${labels[mode]} (unavailable)`, detail: reason, disabled: true }
      : { value: mode, label: labels[mode], detail: descriptions[mode] };
  });
}

/** The composer's permissions button name (P1-10): "Permissions: <mode>",
 *  with "(unrestricted)" for a danger mode in place of the label's own
 *  "(danger)" tag, so a screen reader hears what the button is for. */
export function permissionsTriggerLabel(label: string, danger: boolean): string {
  const mode = label.replace(/\s*\(danger\)$/, '');
  return `Permissions: ${mode}${danger ? ' (unrestricted)' : ''}`;
}

/** A mode as the danger confirm names it (lane S re-review of #1304): both
 *  names where the composer's access chip and the pickers differ ("Full
 *  access (Bypass all)"), so whichever surface opened it, the dialog names
 *  the mode that was picked. */
export function permModeConfirmName(label: string, short: string): string {
  const long = label.replace(/\s*\(danger\)$/, '');
  return short === long ? long : `${short} (${long})`;
}

/** The line under a picker for the current mode: its description, or why it
 *  can't run with this agent. */
export function permModeNote(label: string, description: string, caps: Capabilities, mode: string): string {
  const reason = permModeUnavailable(caps, mode);
  return reason ? `${label} · unavailable: ${reason}` : `${label} · ${description}`;
}

/** Where a mode's general description would overclaim for one agent (plan
 *  L0/X21 provider copy, PROVIDER_MATRIX "Copy that overclaims" 3). Every
 *  Hollow run of Antigravity is headless, and headless `agy` can't ask, so
 *  `--mode accept-edits` denies every file write and command it would prompt
 *  for (`agent.rs` `antigravity_args`, agy 1.2.14 probe 2026-10-02). */
export const AGENT_PERM_MODE_DESC: Record<string, Record<string, string> | undefined> = {
  antigravity: {
    acceptEdits: 'Antigravity can’t ask for approval while it runs, so in Edits only it can read but can’t edit files or run commands. Choose Bypass all if it should make changes.',
  },
};

/** The mode descriptions to show for `agent`: the general ones, with that
 *  agent's corrections. */
export function permModeDescriptions<M extends string>(descriptions: Record<M, string>, agent: string | null | undefined): Record<M, string> {
  const own = agent ? AGENT_PERM_MODE_DESC[agent] : undefined;
  return own ? { ...descriptions, ...own } as Record<M, string> : descriptions;
}
