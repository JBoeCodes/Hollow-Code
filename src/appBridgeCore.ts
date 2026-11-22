/**
 * The pure half of the webview bridge (`appBridge.ts`), loadable in the
 * node vitest suite: event shapes, refresh plans, the UI-owned setting keys
 * (the Ui rows of `src-tauri/src/settings_keys.rs`; a vitest guard holds
 * them together) and the request answerer, over any settings table.
 */
import { GO_TARGETS, NAVIGATE_PAGES } from './keyboardLeader';

/** `app-session-started`, from `app_sessions::queue_turn`. */
export interface AppSessionStarted {
  sessionKey: string;
  workspaceId: string | null;
  cwd: string;
  /** The session (or automation) that asked. */
  startedBy: string;
}

/** `app-changed`, from `app_broker::execute`. */
export interface AppChanged {
  action: string;
  area: string;
}

/** What to refresh for one event. */
export interface RefreshPlan {
  rail: string | null;
  workspaces: boolean;
  sessions: boolean;
  themes?: boolean;
  /** Backend settings a Settings panel holds in its own read (Learning
   *  suggestions, the extraction model, Retitle new chats): those panels
   *  read them again (`store/settingsRevision.ts`, overnight H6). */
  settings?: boolean;
}

export function planForStarted(e: AppSessionStarted): RefreshPlan {
  return { rail: e.workspaceId || null, workspaces: true, sessions: true };
}

/** The actions that change a setting a Settings panel holds in its own
 *  read, besides every `settings.extraction` action: Learning suggestions,
 *  and `settings.change`, which can change any key (`learning.suggestions`
 *  among them). Not the whole `learning` area: proposing a learning changes
 *  no setting. */
export const SETTINGS_CHANGING_ACTIONS: readonly string[] = ['learning.set_learning_suggestions', 'settings.change'];

export function planForChanged(e: AppChanged): RefreshPlan {
  return {
    rail: null,
    workspaces: e.area === 'workspaces' || e.area === 'build',
    sessions: e.area === 'sessions' || e.area === 'workspaces',
    themes: e.area === 'settings.appearance',
    settings: e.area === 'settings.extraction' || SETTINGS_CHANGING_ACTIONS.includes(e.action),
  };
}

/** Every UI-owned setting key. */
export const UI_SETTING_KEYS = [
  'agentTools.vaultSearch', 'agentTools.codeContext', 'agentTools.computerControl', 'agentTools.teamwork',
  'agentRole.claude', 'agentRole.codex', 'agentRole.antigravity', 'agentRole.grok',
  'operator.contextEnabled', 'semanticSearchEnabled', 'claudeQuotaOptIn',
  'app.runOnStartup', 'app.minimizeToTray', 'heartbeat.enabled', 'heartbeat.drafts', 'heartbeat.config',
  'theme', 'notifyInApp', 'notifyNative', 'nudgesEnabled', 'sidebarCollapsed', 'chatPanelOpen',
  'sidePaneWidth', 'chatContextEnabled', 'railCollapsed', 'notifFilter', 'paletteRecents',
  'backupManagedElsewhere',
] as const;
export type UiSettingKey = typeof UI_SETTING_KEYS[number];

/** A UI setting: its store getter and setter. */
export interface UiSetting { get: () => unknown; set: (v: unknown) => unknown }

export const bool = (v: unknown): boolean => {
  if (typeof v !== 'boolean') throw new Error('The value must be true or false.');
  return v;
};
export const ids = (v: unknown): string[] => {
  const list = (v as { ids?: unknown } | null)?.ids;
  if (!Array.isArray(list) || !list.every(x => typeof x === 'string')) throw new Error('The value must be {ids: [strings]}.');
  return list;
};
/** Set a toggle-only signal to `on`. */
export const toggleTo = (current: () => boolean, toggle: () => void) => (v: unknown) => { if (current() !== bool(v)) toggle(); };

export interface AppUiRequest { id: string; op: string; args: Record<string, unknown> }
export type AppUiAnswer = { ok: true; value: unknown } | { ok: false; error: string };

/** Pages `ui.navigate` once took that are gone, each with the page to use. */
const RETIRED: Readonly<Record<string, string>> = {
  calendar: 'tasks', projects: 'home', people: 'library', graph: 'library', ingest: 'library', imports: 'library',
  today: 'home', 'weekly-review': 'home', agenda: 'tasks', commitments: 'tasks', inbox: 'waiting',
  map: 'home', runbooks: 'automations', 'whats-new': 'home',
};
/** Page ids it once took that have go-to names now. */
const RENAMED: Readonly<Record<string, string>> = { vault: 'library', workflows: 'automations', review: 'waiting', chat: 'history' };

/** Names the person sees that `navigate` also takes (P1-05); Library and
 *  Automations already are page ids. */
export const NAVIGATE_ALIASES: Readonly<Record<string, string>> = { 'your-work': 'home' };

/** The page `navigate` opens for `page` (P0-06): one of NAVIGATE_PAGES
 *  (keyboardLeader.ts), or an alias for one. A retired or renamed id is
 *  refused with the one to use, never quietly redirected. */
export function navigateTarget(page: string): string {
  if (Object.hasOwn(NAVIGATE_ALIASES, page)) return NAVIGATE_ALIASES[page];
  if (Object.hasOwn(RETIRED, page)) throw new Error(`navigate: ${page} is retired; use ${RETIRED[page]}.`);
  if (Object.hasOwn(RENAMED, page)) throw new Error(`navigate: ${page} is now ${RENAMED[page]}.`);
  if (!NAVIGATE_PAGES.includes(page)) throw new Error(`navigate: unknown page ${page}.`);
  return page;
}

/** Where a go-to target lands, as the person reads it: Work's views by
 *  their tab, Conversation history by its Library tab (S4), the rest by their
 *  name (P1-05: `navigate` answers with this). Today, Weekly review and the
 *  workspace map were cut (overnight 11-08 S3 and S4). Running is its own page. */
const WORK_PATHS: Readonly<Record<string, string>> = {
  home: 'Projects → Home',
  'mission-control': 'Projects → Running', 'ask-vault': 'Projects → Home → Ask anything',
  history: 'Library → Conversations',
};
export function visiblePath(page: string, record?: string | null): string {
  const base = (Object.hasOwn(WORK_PATHS, page) ? WORK_PATHS[page] : null) ?? GO_TARGETS.find(t => t.id === page)?.label ?? page;
  return record ? `${base} → ${record}` : base;
}

/** Answer one request with `settings` and `navigate`. */
export async function answerWith(
  settings: Record<string, UiSetting>,
  navigate: (page: string, record?: string | null) => Promise<string>,
  req: AppUiRequest,
): Promise<AppUiAnswer> {
  try {
    const args = req.args ?? {};
    switch (req.op) {
      case 'get': {
        const keys = Array.isArray(args.keys) ? args.keys as string[] : Object.keys(settings);
        return { ok: true, value: Object.fromEntries(keys.map(k => [k, settings[k] ? settings[k].get() : null])) };
      }
      case 'set': {
        const key = String(args.key ?? '');
        const setting = settings[key];
        if (!setting) throw new Error(`${key} is not a UI setting.`);
        const old = setting.get();
        await setting.set(args.value);
        return { ok: true, value: { old, new: setting.get() } };
      }
      case 'navigate':
        return { ok: true, value: await navigate(String(args.page ?? ''), args.record as string | undefined) };
      default:
        throw new Error(`unknown request ${req.op}`);
    }
  } catch (e) {
    return { ok: false, error: e instanceof Error ? e.message : String(e) };
  }
}

/** What `take_pending_ui_settings` returns: each queued setting and its
 *  version. Nothing is cleared until the versions are acknowledged. */
export interface PendingUiSettings {
  settings?: Record<string, unknown> | null;
  versions?: Record<string, number> | null;
}

/** Apply what the COO queued while Hollow's window was closed (Rust
 *  `take_pending_ui_settings`), each through the same store setter a live
 *  `set` uses. A key the webview doesn't own, or a value its setter refuses,
 *  is skipped and the rest still apply. Then `ack` (Rust
 *  `ack_pending_ui_settings`) gets every entry it settled, applied or
 *  dropped, with its version: the backend removes only those, so an entry
 *  survives a reload mid-drain and one queued again meanwhile waits for the
 *  next drain. Resolves with the keys applied. */
export async function applyPendingWith(
  settings: Record<string, UiSetting>,
  take: () => Promise<PendingUiSettings | null | undefined>,
  ack?: (versions: Record<string, number>) => Promise<unknown>,
): Promise<string[]> {
  const queued = (await take()) ?? {};
  const entries = queued.settings ?? {};
  const versions = queued.versions ?? {};
  const applied: string[] = [];
  const settled: Record<string, number> = {};
  for (const [key, value] of Object.entries(entries)) {
    const setting = Object.hasOwn(settings, key) ? settings[key] : undefined;
    if (setting) {
      try {
        await setting.set(value);
        applied.push(key);
      } catch { /* a refused value is dropped; the others still apply */ }
    }
    settled[key] = Object.hasOwn(versions, key) && typeof versions[key] === 'number' ? versions[key] : 0;
  }
  if (ack && Object.keys(settled).length > 0) await ack(settled);
  return applied;
}

/** Run bridge work one piece at a time: a drain of the queue and a live
 *  request never interleave, so a queued value can't land after a newer live
 *  set (or the other way round). A failed piece doesn't stop the next. */
export function serializer(): <T>(work: () => Promise<T>) => Promise<T> {
  let tail: Promise<unknown> = Promise.resolve();
  return <T>(work: () => Promise<T>): Promise<T> => {
    const next = tail.then(work, work);
    tail = next.catch(() => undefined);
    return next;
  };
}
