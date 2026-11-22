/**
 * The canvas host contract (plan L3 X7/X8, 2026-10-14; review U canvases).
 *
 * One saved canvas shows in six places: inline in a reply, expanded in the
 * Preview pane, in Results, in the gallery (Library → Results → Canvases),
 * pinned to Home or a workspace, and as an older revision opened in any of
 * those. (A seventh host, the curated section view, went with the workspace
 * panel in COO rebuild L4.) Each used to decide its own chrome, so a
 * version label, a saved state or a permission boundary showed in one host
 * and not the next. This table is the one answer: for every host, every
 * control is either offered (and where) or withheld (and why). The hosts
 * render through `components/HtmlArtifact.tsx`'s shared head, and
 * `hostContract.test.ts` checks the table; the words a head shows come from
 * the helpers below, so a host can't drift into its own phrasing.
 *
 * Pure: no imports, so any host (and the node suite) can read it.
 */

export type CanvasHost = 'inline' | 'expanded' | 'result' | 'gallery' | 'pinned' | 'older';
export type HostControl =
  | 'title' | 'version' | 'saved' | 'expand' | 'pin' | 'compare' | 'copy'
  | 'permission' | 'readAsText' | 'answer' | 'accept' | 'export';

export const CANVAS_HOSTS: readonly CanvasHost[] = ['inline', 'expanded', 'result', 'gallery', 'pinned', 'older'];
export const HOST_CONTROLS: readonly HostControl[] = [
  'title', 'version', 'saved', 'expand', 'pin', 'compare', 'copy', 'permission', 'readAsText', 'answer', 'accept', 'export',
];

/** Offered, and where the person finds it; or withheld, and why. */
export type Offer = { offered: true; where: string } | { offered: false; why: string };
export interface HostTerms {
  /** What the host is, in a few words. */
  name: string;
  controls: Readonly<Record<HostControl, Offer>>;
}

const on = (where: string): Offer => ({ offered: true, where });
const off = (why: string): Offer => ({ offered: false, why });

/** Shared by every host that draws through HtmlArtifact's head. */
const HEAD = {
  title: on('Head: the CANVAS badge and title; the frame is named "Canvas: <title>".'),
  copy: on('⋯ → View source (⌥⌘U) and Copy source.'),
  export: on('⋯ → Export canvas… (a canvas in a workspace or Chats), drawn off screen into outputs/exports; Show the file opens the saved file in the Preview pane.'),
  permission: on('Head tag before anything is refused (Scripts on · no internet, …); one permission line after a refusal.'),
  readAsText: on('⋯ → Read as text: the canvas’s words, headings, lists and tables, in the page.'),
} as const;

const NO_ACCEPT_HERE = off('Accept belongs to a workspace’s Results, where it records the current version.');

export const HOST_CONTRACT: Readonly<Record<CanvasHost, HostTerms>> = {
  inline: {
    name: 'A canvas in a reply',
    controls: {
      ...HEAD,
      version: on('Head: vN, or "vN of M" on a reply whose fence is an earlier version; with more than one kept version the label opens the list.'),
      saved: on('Head: Saved in this workspace / with this conversation, Not kept (older than the kept versions) or Not saved. Failed state saves offer Try again; read-only changes are labelled changes not kept.'),
      expand: on('Expand opens it full height in the Preview pane; without one it grows in place.'),
      pin: on('Pin menu in the head, on the current version only.'),
      compare: on('⋯ → Compare versions in the gallery (a workspace canvas with more than one kept version).'),
      answer: on('An ask-canvas’s Send fills the composer and never sends; only from the current version.'),
      accept: NO_ACCEPT_HERE,
    },
  },
  expanded: {
    name: 'The same canvas in the Preview pane (Expand)',
    controls: {
      ...HEAD,
      title: on('ActionMenu first line and frame aria-label name the canvas.'),
      version: on('ActionMenu first line and frame aria-label; ⋯ → Versions and pin… opens the same version list as the reply.'),
      saved: on('ActionMenu first line and frame aria-label: the same saved state as the reply; the panel copy records nothing.'),
      permission: on('Corner lock with accessible name and tooltip; globe when internet is allowed. The refusal line stays visible.'),
      expand: off('It is already full height in the Preview pane.'),
      pin: on('⋯ → Versions and pin… → Pin, on the current version only.'),
      compare: on('⋯ → Compare versions in the gallery. The pane has no room for two frames side by side.'),
      answer: on('An ask-canvas’s Send fills the open chat’s composer and never sends; only from the current version.'),
      accept: NO_ACCEPT_HERE,
    },
  },
  result: {
    name: 'A saved canvas in a workspace’s Results',
    controls: {
      ...HEAD,
      version: on('Head: vN; with more than one kept version the label opens the list.'),
      saved: on('Head: Saved in this workspace.'),
      expand: off('Results already shows it full height. ⋯ → Open source chat reaches the displayed version’s recorded conversation.'),
      pin: on('Pin menu in the head, on the current version only.'),
      compare: on('⋯ → Compare versions in the gallery.'),
      answer: off('Results only reads a canvas; an ask-canvas is answered in its conversation.'),
      accept: on('Results’ Accept records the current version, and is held back while an older version shows.'),
    },
  },
  gallery: {
    name: 'Library → Results → Canvases',
    controls: {
      ...HEAD,
      version: on('Head: vN and the version list; the list item says the workspace, version and when it was updated.'),
      saved: on('Head: Saved in this workspace · updated <when>.'),
      expand: off('The gallery already shows it full height. ⋯ → Open source chat reaches the displayed version’s recorded conversation, including comparisons.'),
      pin: on('Pin menu in the canvas head, once.'),
      compare: on('Compare versions: two versions side by side and what changed in the source.'),
      answer: off('The gallery only reads a canvas; an ask-canvas is answered in its conversation.'),
      accept: off('Accept is in the workspace’s Results; Open workspace goes there.'),
    },
  },
  pinned: {
    name: 'A canvas pinned to Home or a workspace',
    controls: {
      ...HEAD,
      export: off('A pin shows live values on this Mac; save the canvas from Results or the gallery.'),
      version: on('Head: Pinned vN · current, or Pinned vN · a newer version exists.'),
      saved: on('Status: Live · updated, Saved snapshot, Waiting to refresh or Needs refresh, with the time.'),
      expand: on('Expand grows it in place; Open source chat opens where it was made.'),
      pin: on('Unpin. Update to vN replaces a stale pin only after the new revision and its hash are verified. Pin menus offer Replace and Unpin; success shows a four-second notice with Show.'),
      compare: off('A pin keeps one exact version; compare versions in the gallery.'),
      permission: on('Head tag: Read-only local data. A pin gets only values Hollow reads on this Mac and never inherits an internet permission.'),
      answer: off('A pin is a snapshot; an ask-canvas is answered in its source conversation (Open source chat).'),
      accept: NO_ACCEPT_HERE,
    },
  },
  older: {
    name: 'An older version opened in any host',
    controls: {
      ...HEAD,
      copy: on('⋯ → View source and Copy source copy this older version exactly.'),
      version: on('Head: Older version (vN of M) · read only, and "Back to vN".'),
      saved: on('Head: the saved state of the kept version. After a state change: Changes here aren’t kept. Open vN to keep them; the head adds changes not kept.'),
      expand: on('As its host: Expand in a reply, full height elsewhere.'),
      pin: off('Only the current version can be pinned.'),
      compare: on('Compare versions in the gallery.'),
      answer: off('An older version never drafts: Send says it isn’t the current version.'),
      accept: off('Accept records the current version only; it is held back while an older one shows.'),
    },
  },
};

/** Whether `host` offers `control`. */
export function offers(host: CanvasHost, control: HostControl): boolean {
  return HOST_CONTRACT[host].controls[control].offered;
}

/* ── The words a head shows ───────────────────────────────────────────── */

/** The store keeps a canvas's newest 30 versions (`MAX_REVISIONS` in
 *  `workspace_canvases.rs`). */
export const KEPT_VERSIONS = 30;
export const CHANGES_NOT_KEPT = 'Changes here aren’t kept.';

export type PermissionMode = 'offline' | 'canvas' | 'images' | 'https' | 'interactive';
export type PermissionKind = 'local' | 'none' | 'scripts' | 'internet';
export interface PermissionTag { kind: PermissionKind; text: string; detail: string }

/**
 * The permission boundary in a head, shown before anything is refused. The
 * same grant state as the permission line (`canvasGrants.ts`): `mode` is the
 * policy the frame renders in now, `base` the one it opens in, so a raised
 * policy is the person's own grant. `localOnly`: a pinned canvas, which only
 * ever gets local values. `what`: canvas, HTML or graphic.
 */
export function permissionTag(mode: PermissionMode, base: PermissionMode, localOnly: boolean, what = 'canvas'): PermissionTag {
  if (localOnly) {
    return { kind: 'local', text: 'Read-only local data', detail: `This pinned ${what} gets only values Hollow reads on this Mac. It can’t reach the internet or change anything.` };
  }
  if (mode === 'offline') return { kind: 'none', text: 'No scripts · no internet', detail: `This ${what} runs no scripts and can’t reach the internet.` };
  if (mode === 'canvas') return { kind: 'scripts', text: 'Scripts on · no internet', detail: `This ${what}’s scripts run in a sandbox, apart from Hollow. It can’t reach the internet. ⋯ → Allow internet to change that.` };
  const allowed = mode !== base;
  return {
    kind: 'internet',
    text: allowed ? 'Internet on (you allowed it)' : 'Internet on',
    detail: `This ${what} may load from and send to any website. ⋯ → Turn internet off.`,
  };
}

/** Where a saved canvas lives: a workspace, Quick chats, or a conversation
 *  outside any workspace (app data). Null: no store yet (a new chat). */
export type CanvasStoreKind = 'workspace' | 'quick' | 'conversation' | null;

/**
 * A canvas's saved state in words (inline, expanded, older). `saved`: this
 * fence's body is one of the store's kept versions. `revision`: the version
 * the store matched it to even when not kept (an aged-out body, the store's
 * `retired`). `latest`: the store's current version when it has the canvas.
 */
export function savedWords(s: { store: CanvasStoreKind; saved: boolean; revision: number | null; latest: number | null }): { text: string; detail: string } {
  if (s.saved) {
    if (s.store === 'conversation') return { text: 'Saved with this conversation', detail: 'Kept in Hollow’s app data with this conversation, with its last 30 versions.' };
    if (s.store === 'quick') return { text: 'Saved with the COO', detail: 'Kept in the COO’s chats folder (outputs/canvases), with its last 30 versions.' };
    return { text: 'Saved in this workspace', detail: 'Kept in this workspace’s outputs/canvases, with its last 30 versions.' };
  }
  if (s.revision !== null && s.latest !== null) {
    return { text: 'Not kept', detail: `Older than this canvas’s last ${KEPT_VERSIONS} versions, which are all the store keeps; shown from the conversation.` };
  }
  return { text: 'Not saved', detail: 'Shown from the conversation; it isn’t in a canvas store.' };
}

/** An older version on screen (X8): every host that can show one says so. */
export function olderLabel(shown: number, latest: number): string {
  return `Older version (v${shown} of ${latest}) · read only`;
}

/** Whether `shown` is an older version than the current one. */
export function isOlder(shown: number | null, latest: number | null): boolean {
  return shown !== null && latest !== null && shown < latest;
}

/** A pin's version in its head. `newer`: null until the store was read. */
export function pinnedVersionLabel(revision: number, newer: boolean | null): string {
  if (newer === null) return `Pinned v${revision}`;
  return newer ? `Pinned v${revision} · a newer version exists` : `Pinned v${revision} · current`;
}


