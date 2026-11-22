/**
 * 10-11 B1: the sandbox policy a canvas opens in, chosen for the person.
 *
 * Every HTML surface used to show five policy buttons (Offline, Canvas,
 * Images, HTTPS, Interactive). Now a document opens in its default (Canvas
 * for what an agent wrote in Hollow, Offline for HTML from outside) and asks
 * for more only when the browser actually refused something: the frame's
 * runtime reports the refusal (bridge `blocked`), the host shows one
 * permission line, and **Allow** raises the policy just enough
 * (`escalate`). The grant is remembered here, in localStorage, for that
 * canvas *and its exact content*: a new revision, or an edited file, asks
 * again (review: a grant given for a chart CDN must not carry over to text
 * written later). The five policies themselves, and what
 * `artifacts.rs` serves for each, are unchanged.
 */
import type { ArtifactMode } from '../components/htmlSandbox';
import type { BlockedWant } from '../canvasBridge';

export const GRANTS_KEY = 'hollow.canvasGrants.v1';
/** Newest grants kept; older ones are forgotten (the canvas asks again). */
export const GRANTS_CAP = 300;

const MODES: readonly ArtifactMode[] = ['offline', 'canvas', 'images', 'https', 'interactive'];

/** Whether a policy runs the document's own scripts. */
const scripted = (m: ArtifactMode) => m === 'canvas' || m === 'interactive';

/** The smallest policy above `mode` that allows what the frame wanted.
 *  Anything scripted that wants the network gets Interactive: a page that
 *  runs scripts and may load remote images can already send data out in an
 *  image URL, so the permission is honestly "reach the internet". */
export function escalate(mode: ArtifactMode, want: BlockedWant): ArtifactMode {
  if (want === 'scripts') return mode === 'offline' ? 'canvas' : mode === 'images' || mode === 'https' ? 'interactive' : mode;
  if (scripted(mode)) return 'interactive';
  if (want === 'images') return mode === 'offline' ? 'images' : mode;
  return 'https';
}

/** Whether `mode` already allows `want` (then a refusal can't be ours to grant). */
export function allows(mode: ArtifactMode, want: BlockedWant): boolean {
  return escalate(mode, want) === mode;
}

/** The internet switch in a canvas's menu: the policy with the network on,
 *  keeping scripts as they are. */
export function withInternet(mode: ArtifactMode): ArtifactMode {
  return scripted(mode) ? 'interactive' : 'https';
}

/** The internet switch turned off again: scripts as they are, no network. */
export function withoutInternet(mode: ArtifactMode): ArtifactMode {
  return scripted(mode) ? 'canvas' : 'offline';
}

/** What a permission line calls the document: a saved canvas, an html
 *  fence in a reply, or an `.html` file. */
export type BlockedWhat = 'canvas' | 'html' | 'file';

/** The permission line's sentence for a refusal. */
export function blockedLine(want: BlockedWant, host: string, what: BlockedWhat): string {
  const it = what === 'canvas' ? 'This canvas' : what === 'file' ? 'This file' : 'This HTML';
  if (want === 'scripts') {
    return what === 'file'
      ? 'Scripts in this file are off: it didn’t come from an agent in Hollow.'
      : 'Scripts in this HTML are off outside workspaces and Chats.';
  }
  // The host is the frame's own report, and Allow opens every website, not
  // just that one: say so.
  if (want === 'images') return `${it} tried to load images from ${host}. Allowing lets it load from and send to any website.`;
  return `${it} tried to connect to ${host}. Allowing lets it load from and send to any website.`;
}

/** A short stable key for an html fence or file text (FNV-1a, 32 bits). */
export function textKey(text: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return `fence:${h.toString(36)}:${text.length}`;
}

/** A grant: the policy, when it was given, and the content it was given for
 *  (`textKey`). */
type Store = Record<string, { mode: ArtifactMode; at: number; text: string }>;

function read(): Store {
  try {
    const raw = localStorage.getItem(GRANTS_KEY);
    const v: unknown = raw ? JSON.parse(raw) : null;
    if (!v || typeof v !== 'object' || Array.isArray(v)) return {};
    const out: Store = {};
    for (const [k, g] of Object.entries(v as Record<string, unknown>)) {
      const e = g as { mode?: unknown; at?: unknown; text?: unknown } | null;
      if (e && MODES.includes(e.mode as ArtifactMode) && typeof e.at === 'number' && typeof e.text === 'string') out[k] = { mode: e.mode as ArtifactMode, at: e.at, text: e.text };
    }
    return out;
  } catch {
    return {};
  }
}

function write(store: Store) {
  const keep = Object.entries(store).sort((a, b) => b[1].at - a[1].at).slice(0, GRANTS_CAP);
  try { localStorage.setItem(GRANTS_KEY, JSON.stringify(Object.fromEntries(keep))); } catch { /* storage full or off: the grant lasts this view */ }
}

/** The policy `key` opens in with `content`: its grant when that was given
 *  for this exact content, else `base`. A grant never lowers the default: a
 *  canvas that opens in Canvas keeps Canvas even if an older grant says
 *  Images (a policy that runs no scripts). */
export function grantedMode(key: string, base: ArtifactMode, content: string): ArtifactMode {
  const g = read()[key];
  if (!g || g.text !== textKey(content)) return base;
  if (scripted(base) && !scripted(g.mode)) return 'interactive';
  return g.mode;
}

let clock = 0;
/** Remember `mode` for `key` and this `content`; `null` forgets it (the
 *  canvas returns to its default). */
export function setGrant(key: string, mode: ArtifactMode | null, content: string) {
  const store = read();
  if (mode === null) delete store[key];
  else {
    // Strictly increasing, so grants made in one millisecond still order.
    clock = Math.max(Date.now(), clock + 1, ...Object.values(store).map(g => g.at + 1));
    store[key] = { mode, at: clock, text: textKey(content) };
  }
  write(store);
}

/** A workspace's deliverables folder, where agents write their HTML. */
const OUTPUTS = /(^|\/)workspaces\/[^/]+\/outputs\//;
/** A `.` or `..` segment: `outputs/../sources/x.html` is not in outputs. */
const DOT_SEGMENT = /(^|\/)\.{1,2}(\/|$)/;

/** The policy an `.html` file opens in (10-11 B1): Canvas (scripts, no
 *  network) when an agent wrote it in Hollow, either in this conversation
 *  (`agentWrote`: the preview pane's write list) or into a workspace's
 *  `outputs/`; Offline for HTML from anywhere else (sources, downloads,
 *  imports, briefs). */
export function fileMode(path: string, agentWrote: boolean): ArtifactMode {
  if (DOT_SEGMENT.test(path)) return 'offline';
  return agentWrote || OUTPUTS.test(path) ? 'canvas' : 'offline';
}

/** A saved canvas's grant key: the same canvas in a reply, Results and a
 *  curated view shares it (its store: the workspace, else the conversation). */
export function canvasGrantKey(scope: { workspaceId: string | null; sessionKey: string | null }, id: string): string | undefined {
  // No store yet (a new chat): no shared key; the content key applies.
  if (!scope.workspaceId && !scope.sessionKey) return undefined;
  return `canvas:${scope.workspaceId ?? `session:${scope.sessionKey}`}:${id}`;
}
