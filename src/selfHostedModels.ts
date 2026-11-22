/**
 * Pure helpers for Settings, Models, Self-hosted models (self-hosted
 * models Q4, `self_hosted_registry.rs`): the add/edit form's draft, the
 * token command's arguments as one line, and the words around an entry and
 * its Test. The backend checks every rule on save; the form only turns its
 * text into an entry. Type-only imports, so the node vitest suite can load it.
 *
 * Q10: the Background jobs block's words and choices (`self_hosted_jobs.rs`):
 * each job kind's select and the line for jobs waiting for a model.
 */
import type { SelfHostedEntry, SelfHostedHeldJob, SelfHostedJobRow, SelfHostedRetry, SelfHostedTest, SelfHostedWaiting } from './generated/ipc';

/** An entry's API style, which decides the agent that runs it (Q9). */
export type ApiStyle = 'anthropic' | 'openai';

/** The form's fields as typed. */
export interface SelfHostedDraft {
  id: string;
  name: string;
  baseUrl: string;
  /** Anthropic's Messages API (Claude) or OpenAI's Responses API (Codex). */
  apiStyle: ApiStyle;
  servedModel: string;
  /** Tokens as typed ("131072" or "131,072"). */
  contextWindow: string;
  effortLevels: string[];
  authKind: 'tokenCommand' | 'none';
  program: string;
  /** The token command's arguments on one line (`formatArgs`). */
  args: string;
  healthPath: string;
  enabled: boolean;
}

/** Shown while a Test runs: the health check wakes a cold model first. */
export const TEST_RUNNING = 'Starting the model, this can take about 5 minutes.';

/** The window's limits when the list hasn't said (`MIN_CONTEXT_WINDOW`
 *  and `MAX_CONTEXT_WINDOW` in `self_hosted_registry.rs`). */
export const WINDOW_LIMITS = { min: 110_000, max: 10_000_000 } as const;

export const AUTH_OPTIONS = [
  { value: 'tokenCommand', label: 'Token command' },
  { value: 'none', label: 'None' },
] as const;

/** Self-hosted models Q9: the API styles and the agent each runs on
 *  (`self_hosted::harness`). */
export const API_STYLE_OPTIONS = [
  { value: 'anthropic', label: "Anthropic's Messages API (Claude)" },
  { value: 'openai', label: "OpenAI's Responses API (Codex)" },
] as const;

/** The levels an OpenAI-style entry may offer: Codex's own, which has no
 *  `max` (`self_hosted_registry::CODEX_LEVELS`). */
export const CODEX_LEVELS: readonly string[] = ['low', 'medium', 'high', 'xhigh'];

/** The style as the form holds it (anything but `openai` reads as
 *  `anthropic`, the registry's default). */
export function apiStyleOf(style: string): ApiStyle {
  return style === 'openai' ? 'openai' : 'anthropic';
}

/** The agent an entry's API style runs on. */
export function harnessOf(style: string): 'Claude' | 'Codex' {
  return apiStyleOf(style) === 'openai' ? 'Codex' : 'Claude';
}

/** The effort levels the form offers for `style`, from the list's own. */
export function levelsFor(style: ApiStyle, levels: readonly string[]): string[] {
  return style === 'openai' ? levels.filter(level => CODEX_LEVELS.includes(level)) : [...levels];
}

/** The draft with its API style changed: an OpenAI-style entry drops the
 *  levels Codex doesn't send. */
export function withApiStyle(draft: SelfHostedDraft, style: ApiStyle): SelfHostedDraft {
  return { ...draft, apiStyle: style, effortLevels: levelsFor(style, draft.effortLevels) };
}

export function emptyDraft(): SelfHostedDraft {
  return {
    id: '', name: '', baseUrl: '', apiStyle: 'anthropic', servedModel: '', contextWindow: '',
    effortLevels: ['low', 'medium', 'high'], authKind: 'tokenCommand',
    program: '', args: '', healthPath: '/health', enabled: true,
  };
}

/** An argument as typed on the one line: quoted when it has a space, a
 *  quote or a backslash, or is empty. */
function formatArg(arg: string): string {
  return /^[^\s"\\]+$/.test(arg) ? arg : `"${arg.replace(/(["\\])/g, '\\$1')}"`;
}

export function formatArgs(args: readonly string[]): string {
  return args.map(formatArg).join(' ');
}

/** The one line back into arguments: spaces separate them, double quotes
 *  keep spaces inside one, and a backslash takes the next character as is. */
export function parseArgs(text: string): { args: string[] } | { error: string } {
  const args: string[] = [];
  let current = '';
  let started = false;
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '\\' && i + 1 < text.length) { current += text[++i]; started = true; continue; }
    if (c === '"') { quoted = !quoted; started = true; continue; }
    if (!quoted && /\s/.test(c)) {
      if (started) args.push(current);
      current = ''; started = false;
      continue;
    }
    current += c; started = true;
  }
  if (quoted) return { error: 'A quote in the arguments is not closed.' };
  if (started) args.push(current);
  return { args };
}

export function draftFrom(entry: SelfHostedEntry): SelfHostedDraft {
  return {
    id: entry.id, name: entry.name, baseUrl: entry.baseUrl, apiStyle: apiStyleOf(entry.apiStyle), servedModel: entry.servedModel,
    contextWindow: String(entry.contextWindow), effortLevels: [...entry.effortLevels],
    authKind: entry.auth.kind,
    program: entry.auth.kind === 'tokenCommand' ? entry.auth.program : '',
    args: entry.auth.kind === 'tokenCommand' ? formatArgs(entry.auth.args) : '',
    healthPath: entry.healthPath, enabled: entry.enabled,
  };
}

/** The entry the form describes, or what keeps it from being one: the
 *  window a whole number within `limits` (the list's own), so it never
 *  reaches the backend as a number it can't read. Every other rule is the
 *  backend's (`self_hosted_registry::normalize`). */
export function entryFrom(
  draft: SelfHostedDraft,
  limits: { min: number; max: number } = WINDOW_LIMITS,
): { entry: SelfHostedEntry } | { error: string } {
  const window = draft.contextWindow.replace(/[,_\s]/g, '');
  if (!/^\d+$/.test(window)) return { error: 'The context window must be a whole number of tokens.' };
  const tokens = Number(window);
  if (tokens < limits.min || tokens > limits.max) {
    const n = (v: number) => v.toLocaleString('en-US');
    return { error: `The context window must be between ${n(limits.min)} and ${n(limits.max)} tokens: Claude Code compacts at 100,000 tokens at the earliest, and a Hollow chat turn's first call is about 52,000.` };
  }
  let auth: SelfHostedEntry['auth'] = { kind: 'none' };
  if (draft.authKind === 'tokenCommand') {
    const parsed = parseArgs(draft.args);
    if ('error' in parsed) return parsed;
    auth = { kind: 'tokenCommand', program: draft.program.trim(), args: parsed.args };
  }
  return {
    entry: {
      id: draft.id.trim(), name: draft.name.trim(), baseUrl: draft.baseUrl.trim(), apiStyle: draft.apiStyle,
      servedModel: draft.servedModel.trim(), contextWindow: tokens, effortLevels: [...draft.effortLevels],
      auth, healthPath: draft.healthPath.trim(), enabled: draft.enabled,
    },
  };
}

/** Self-hosted models Q5 review: the row line for a saved entry that no
 *  longer passes the checks a save makes (a hand-edited file). Chats refuse
 *  it with the same reason until an Edit saves it again. */
export function invalidText(why: string): string {
  return `Chats can't use it as saved: ${why.trim()} Edit it to fix this.`;
}

/** "131,072 tokens". */
export function windowText(tokens: number): string {
  return `${tokens.toLocaleString('en-US')} tokens`;
}

/** Self-hosted models Q9 review: what Remove's confirmation says. An
 *  OpenAI-style model's Codex home, with its Codex conversations, stays in
 *  Hollow's app data: Hollow never deletes it. */
export function removeBody(entry: SelfHostedEntry): string {
  const base = 'Hollow forgets this endpoint and stops its local forwarder. The model and its server are not changed.';
  if (apiStyleOf(entry.apiStyle) !== 'openai') return base;
  return `${base} Its Codex home, with this model's Codex conversations, stays in Hollow's app data (self-hosted-codex/${entry.id}); delete that folder yourself if you no longer want them.`;
}

/** The row's second line: served model, window, how it signs in and (Q9)
 *  the agent it runs on. */
export function entrySummary(entry: SelfHostedEntry): string {
  const signIn = entry.auth.kind === 'tokenCommand' ? `token from ${entry.auth.program}` : 'no sign-in';
  return `${entry.servedModel} · ${windowText(entry.contextWindow)} · ${signIn} · runs on ${harnessOf(entry.apiStyle)}`;
}

/** "812 ms", "4.2 s", "4 min 32 s". Rounded first, then split, so 299.6 s
 *  is "5 min", never "4 min 60 s", and 59.96 s is "1 min", never "60.0 s". */
export function formatMs(ms: number): string {
  const whole = Math.round(ms);
  if (whole < 1000) return `${whole} ms`;
  const tenths = Math.round(ms / 100);
  if (tenths < 600) return `${(tenths / 10).toFixed(1)} s`;
  const seconds = Math.round(ms / 1000);
  const minutes = Math.floor(seconds / 60);
  return seconds % 60 ? `${minutes} min ${seconds % 60} s` : `${minutes} min`;
}

/** What a Test result is about: the entry without its on/off switch, so a
 *  result shows only while the entry it tested is unchanged (a switch is
 *  not a change to the endpoint). */
export function testKey(entry: SelfHostedEntry): string {
  return JSON.stringify({ ...entry, enabled: undefined });
}

/** The model list line: unread (no answer, not 2xx, not a list) or a list
 *  that doesn't name the served model; null when it names it. */
function modelListLine(result: SelfHostedTest, entry: SelfHostedEntry): string | null {
  if (result.modelListed === true) return null;
  if (result.modelListed === false) return `The endpoint's model list doesn't name ${entry.servedModel}.`;
  const status = result.modelsStatus;
  if (status == null) return "Couldn't read the model list.";
  if (status < 200 || status >= 300) return `Couldn't read the model list (${status}).`;
  return "Couldn't read the model list: the answer isn't a list of models.";
}

/** A Test's outcome in words: a state for the chip and one line per fact. */
export function testSummary(result: SelfHostedTest, entry: SelfHostedEntry): { state: 'ok' | 'warn' | 'error'; chip: string; lines: string[] } {
  const lines: string[] = [];
  if (result.healthStatus != null) {
    lines.push(`Health check ${result.healthStatus} in ${formatMs(result.healthMs ?? 0)}.`);
  }
  if (result.status != null && result.firstByteMs != null) {
    lines.push(`First byte ${result.status} in ${formatMs(result.firstByteMs)}.`);
  }
  const reached = result.healthStatus != null && result.healthStatus < 300;
  const listLine = reached ? modelListLine(result, entry) : null;
  if (listLine) lines.push(listLine);
  if (result.windowMismatch && result.reportedWindow != null) {
    lines.push(`The endpoint reports a window of ${windowText(result.reportedWindow)}; this entry says ${windowText(entry.contextWindow)}.`);
  }
  if (result.error) lines.push(result.error);
  if (!result.ok) return { state: 'error', chip: 'Failed', lines };
  if (result.windowMismatch || listLine) return { state: 'warn', chip: 'Passed with a warning', lines };
  return { state: 'ok', chip: 'Passed', lines };
}

/** A chat on a self-hosted entry has the model id `self-hosted:<entry id>`
 *  (self-hosted models Q5/Q6). */
export const SELF_HOSTED_MODEL_PREFIX = 'self-hosted:';

/** The entry id in a chat's model id (`self-hosted:qwen` → `qwen`), or null
 *  for any other model. The id follows the registry's rule
 *  (`^[a-z0-9][a-z0-9-]{0,31}$`). */
export function selfHostedEntryId(model: string): string | null {
  if (!model.startsWith(SELF_HOSTED_MODEL_PREFIX)) return null;
  const id = model.slice(SELF_HOSTED_MODEL_PREFIX.length);
  return /^[a-z0-9][a-z0-9-]{0,31}$/.test(id) ? id : null;
}

// ── Background jobs (self-hosted models Q10) ─────────────────────────────

/** The select's value for "As now" (the kind runs where it runs today). */
export const AS_NOW = '';

/** A job kind's choices: As now, then every switched-on model Hollow can
 *  run (Anthropic style). The saved model stays listed when it can't run
 *  now (off, removed, another style), so the select shows what is saved. */
export function jobOptions(row: SelfHostedJobRow, entries: readonly SelfHostedEntry[]): { value: string; label: string }[] {
  const options = [{ value: AS_NOW, label: 'As now' }];
  for (const e of entries) {
    if (e.enabled && e.apiStyle === 'anthropic') options.push({ value: e.id, label: e.name });
  }
  if (row.entry && !options.some(o => o.value === row.entry)) {
    const name = entries.find(e => e.id === row.entry)?.name ?? row.entry;
    options.push({ value: row.entry, label: `${name} (can't run now)` });
  }
  return options;
}

/** "3 jobs waiting for Qwen3.8 27B". */
export function waitingLine(w: SelfHostedWaiting): string {
  return `${w.count} ${w.count === 1 ? 'job' : 'jobs'} waiting for ${w.name ?? w.entry}`;
}

/** Whole minutes from `iso` to `now`, at least 0. */
function minutesSince(iso: string, now: Date): number {
  const at = Date.parse(iso);
  return Number.isNaN(at) ? 0 : Math.max(0, Math.floor((now.getTime() - at) / 60_000));
}

/** The waiting line's second line: what Hollow is doing about them. */
export function waitingDetail(w: SelfHostedWaiting, now: Date = new Date()): string {
  if (w.draining) return `Waking ${w.name ?? w.entry} and running them. ${TEST_RUNNING}`;
  const waited = minutesSince(w.oldestAt, now);
  const since = waited < 1 ? 'The oldest came in just now.' : `The oldest has waited ${waited} min.`;
  const when = 'Hollow wakes the model once one has waited 30 minutes, or right after the model answers a chat.';
  return w.lastError ? `${since} The last try stopped: ${w.lastError}` : `${since} ${when}`;
}

/** Self-hosted models Q9 with Q10: background jobs are Claude one-shots, so
 *  only Anthropic-style models are offered for them ({@link jobOptions}); a
 *  saved choice whose model became OpenAI-style is held, never run As now. */
export const JOBS_CLAUDE_ONLY = "They run on Claude, so only models with Anthropic's Messages API are offered here; an OpenAI-style model runs on Codex, in chats only.";

/** Shown before a setting file's error (Q10 review: it fails closed). */
export const JOBS_DAMAGED = 'Background jobs are held until this is fixed or the file is moved aside:';

/** "Recording extraction job held" or "… gave up after 3 tries". */
export function heldLine(h: SelfHostedHeldJob): string {
  return h.gaveUp ? `${h.label} job gave up after ${h.attempts} tries` : `${h.label} job held`;
}

/** What Try again did (Q10 review round 4): its model's run started now,
 *  or one was under way and the next takes the job. Q10 follow-up: the
 *  model is the one the job waits for now (Try again moves a job whose kind
 *  goes elsewhere since), and when Hollow couldn't tell whether a run is
 *  under way it says only that the next one runs it. */
export function retryResult(r: SelfHostedRetry): string {
  const model = r.name ?? r.entry;
  if (r.started) return `✓ Waking ${model} to run it again`;
  return r.underWay
    ? `✓ Back in line: a run of ${model} is under way, and the next one runs it`
    : `✓ Back in line: the next run of ${model} runs it`;
}

/** What a held job waits for, and what each of its buttons does: Try
 *  again for one that gave up while its kind still goes to a model, Run as
 *  now for one whose kind runs As now, Discard always (Q10 review round 2:
 *  the words name only the buttons the row shows). */
export function heldDetail(h: SelfHostedHeldJob, now: Date = new Date()): string {
  const model = h.name ?? h.entry;
  const waited = minutesSince(h.queuedAt, now);
  const since = waited < 1 ? 'Queued just now' : `Queued ${waited} min ago`;
  if (h.running) return `${since} for ${model}. Hollow is running it now.`;
  const error = h.lastError ? ` Last error: ${h.lastError}` : '';
  const failed = `It failed ${h.attempts} ${h.attempts === 1 ? 'time' : 'times'}`;
  if (h.canTryAgain) return `${since} for ${model}. ${failed}: Try again runs it on the model once more (after a run of it already under way), or Discard drops it.${error}`;
  if (h.canRunAsNow) return `${since} for ${model}, and its kind runs As now since. Run as now sends it there, or Discard drops it.${error}`;
  if (h.onlyOn) return `${since} for ${model}. It belongs to a chat on that model, so it runs only there: send its kind to that model again, or Discard drops it.${error}`;
  return `${since} for ${model}. Discard drops it.${error}`;
}
