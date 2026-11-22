// Settings, Context (CX17): the words and states around the five layer rows
// (home, personal, org, project folders, vault). Pure, unit-tested in
// contextLayers.test.ts; rendered by `settings/ContextLayersPanel.tsx`.
import type {
  ContextCheckRepo, ContextCheckStatus, ContextLayersStatus, ContextSeedFolder, ContextRootStatus, ContextSyncStatus, MacContextLayer, MacContextOffer, MacContextReport, MacContextRequest, OrgAccessCheck,
} from './contextLayersApi';

/** A status pill: its words and its `settings-state-chip` state. */
export interface Pill { text: string; state?: 'ok' | 'warn' | 'error' }

/** The personal row's pill. No remote is Local only (a folder, no sync, not
 *  backed up); the sync engine's own line otherwise. */
export function personalPill(s: ContextSyncStatus | undefined): Pill {
  if (!s) return { text: 'Checking…' };
  if (!s.remote) return s.text === 'Not set up' ? { text: 'Not set up' } : { text: 'Local only, not backed up', state: 'warn' };
  switch (s.state) {
    case 'idle': return { text: s.text, state: s.pendingCommits + s.unsyncedFiles > 0 ? 'warn' : 'ok' };
    case 'syncing': return { text: s.text };
    case 'blocked': return { text: s.text, state: 'error' };
    case 'clash': case 'offline': case 'off': return { text: s.text, state: 'warn' };
  }
}

/** Whether the personal row can Sync now or Stop syncing: only with a remote. */
export const personalHasRemote = (s: ContextSyncStatus | undefined) => !!s?.remote;

/** Local only waits while a sync runs (the backend refuses it then too). */
export const canGoLocalOnly = (s: ContextSyncStatus | undefined) => !!s?.remote && s.state !== 'syncing';

/** The mass-removal guard stopped the sync: Sync now can let one sync past
 *  it, after the person confirms. */
export function blockedByRemovals(s: ContextSyncStatus | undefined): boolean {
  if (!s || s.state !== 'blocked') return false;
  return /allow removals/i.test(`${s.blockedBy ?? ''} ${s.text} ${s.lastError ?? ''}`);
}

/** The panel's last good read and its latest error. A failed read keeps the
 *  rows it had (a resource in its error state throws when read, which took
 *  the rows down) and shows the error beside them. */
export interface Loaded<T> { value: T | undefined; error: string }

export function nextLoaded<T>(prev: Loaded<T>, outcome: { ok: true; value: T } | { ok: false; error: unknown }, words: (e: unknown) => string): Loaded<T> {
  return outcome.ok ? { value: outcome.value, error: '' } : { value: prev.value, error: words(outcome.error) };
}

/** What an access check result was about: a different org folder or clone
 *  state makes an old result stale. */
export function orgAccessKey(status: ContextLayersStatus | undefined): string {
  const r = status?.org.root;
  return r ? `${r.path}|${r.configured}|${r.gitClone}` : '';
}

/** The org row's pill: a read-only clone, or what keeps it from being one. */
export function orgPill(root: ContextRootStatus | undefined): Pill {
  if (!root) return { text: 'Checking…' };
  if (!root.configured) return { text: 'Not connected' };
  if (root.problem) return { text: 'Folder needs a look', state: 'error' };
  if (!root.exists) return { text: 'Folder missing', state: 'error' };
  if (!root.gitClone) return { text: 'Not a git clone', state: 'warn' };
  return { text: 'Read only', state: 'ok' };
}

/** The org row's Check access result as a pill. */
export function accessPill(check: OrgAccessCheck): Pill {
  switch (check.state) {
    case 'ok': return { text: 'Access OK', state: 'ok' };
    case 'denied': return { text: 'No access', state: 'error' };
    case 'offline': return { text: 'Offline', state: 'warn' };
    case 'missing': return { text: 'No clone', state: 'warn' };
    default: return { text: 'Check failed', state: 'error' };
  }
}

/** The home row's pill: the machine has a name and its file is there. */
export function homePill(home: ContextLayersStatus['home'] | undefined): Pill {
  if (!home) return { text: 'Checking…' };
  if (!home.slug) return { text: 'Name this Mac', state: 'warn' };
  if (!home.root.configured) return { text: 'No personal clone folder', state: 'warn' };
  if (home.root.problem || !home.root.exists) return { text: 'Folder needs a look', state: 'error' };
  return home.fileExists ? { text: 'Home file found', state: 'ok' } : { text: 'No home file yet', state: 'warn' };
}

export function vaultPill(vault: ContextLayersStatus['vault'] | undefined): Pill {
  if (!vault) return { text: 'Checking…' };
  return vault.setUp ? { text: vault.text, state: 'warn' } : { text: vault.text || 'Not set up' };
}

/** "3 of 12 project folders carry context". */
export function projectsLine(p: ContextLayersStatus['projects'] | undefined): string {
  if (!p) return 'Reading your project folders…';
  if (p.error) return `Couldn’t read the project folders: ${p.error}`;
  if (p.total === 0) return 'No project folders yet.';
  const n = p.withContext.length;
  const of = p.capped ? `the first 60 of ${p.total}` : `${p.total}`;
  return `${n} of ${of} project folder${p.total === 1 ? '' : 's'} carry a CLAUDE.md or AGENTS.md.`;
}

/** Why a remote URL can't be used, before the backend checks it again; null
 *  when it looks usable. Never accepts a URL with credentials in it. */
export function remoteProblem(input: string): string | null {
  const url = input.trim();
  if (!url) return 'Paste the repo’s URL first.';
  if (url.length > 1000 || /\s/.test(url) || url.startsWith('-')) return 'That URL has a space or a leading dash.';
  if (url.includes('::')) return 'Use an https, ssh or git@ URL.';
  const authority = (rest: string) => rest.split('/')[0] ?? '';
  for (const scheme of ['https://', 'http://', 'ssh://']) {
    if (!url.startsWith(scheme)) continue;
    const host = authority(url.slice(scheme.length));
    const at = host.lastIndexOf('@');
    if (scheme !== 'ssh://' && at >= 0) return 'That URL carries a sign-in. Use the plain URL; git signs in on its own.';
    if (scheme === 'ssh://' && at >= 0 && host.slice(0, at).includes(':')) return 'That URL carries a password. Use an SSH key instead.';
    if (scheme === 'http://') return 'Use https, not http.';
    if (!host.slice(at + 1)) return 'That URL names no host.';
    return null;
  }
  if (url.startsWith('/') || url.startsWith('file:///')) return null;
  const colon = url.indexOf(':');
  if (colon > 0) {
    const before = url.slice(0, colon);
    const path = url.slice(colon + 1);
    if (!before.includes('/') && before.includes('@') && path && !path.startsWith('/')) return null;
  }
  return 'Use https://…, ssh://… or git@host:owner/repo.git.';
}

/** Why a machine name can't be saved; null when it can (the backend's rule:
 *  1 to 40 lowercase letters, digits and single dashes, no dash at an end). */
export function slugProblem(input: string): string | null {
  const slug = input.trim();
  if (!slug) return 'Type a name for this Mac.';
  if (slug.length > 40) return 'Keep it to 40 characters.';
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(slug)) return 'Lowercase letters, digits and single dashes only.';
  return null;
}

/** Create, as guided steps: the person makes a private repo, then connects
 *  it. Set it up for me (CX18, below) is the same as a card with one command. */
export const CREATE_STEPS: readonly string[] = [
  'Open GitHub’s new repository page (button below) signed in as yourself.',
  'Name it, for example personal-context, and choose Private. Leave it empty: no README.',
  'Copy its URL (the SSH one, git@github.com:you/personal-context.git, or the https one).',
  'Paste it under Connect an existing repo and press Connect.',
];

export const NEW_REPO_URL = 'https://github.com/new';

// ── Set it up for me (CX18): a card only. Hollow shows the steps and the
// person runs them; nothing here creates, pushes, clones or calls out. ──

export const SETUP_REPO_DEFAULT = 'personal-context';
export const SETUP_OWNER_PLACEHOLDER = '<your-github-login>';
export const SETUP_NOTE = 'Hollow doesn’t create the repo; you run this once.';
export const SETUP_VISIBILITY = 'Private, always: only you can see it.';
export const SETUP_RUN = 'Run this in Terminal, with gh signed in as you:';
export const SETUP_CONNECT = 'Then paste this address under Connect an existing repo and press Connect:';
export const SETUP_CONNECT_FILLED = 'Then press Connect under Connect an existing repo; the address is filled in:';
export const SETUP_OWNER_HINT = 'Put your GitHub login where it says <your-github-login>.';
/** What Connect does today (`configure_personal_sync` saves the address and
 *  turns sync on; the quiet timer runs the first cycle). Nothing more. */
export const SETUP_THEN: readonly string[] = [
  'Connect saves the address and turns sync on.',
  'Within a minute the first sync sends your me, memory, learnings and home folders to it,',
  'and brings back what the repo holds. The rest of the vault never goes there.',
];
export const SETUP_COPIED = 'Command copied. Paste it into Terminal.';

/** A GitHub login as GitHub allows it (1 to 39 letters, digits and single
 *  inner dashes), else null: anything else is treated as unknown. */
export function githubLogin(input: string | null | undefined): string | null {
  const login = (input ?? '').trim();
  return /^[A-Za-z0-9](?:[A-Za-z0-9]|-(?=[A-Za-z0-9])){0,38}$/.test(login) ? login : null;
}

/** Why a repo name can't be used; null when it can: letters, digits, dot,
 *  dash and underscore, at most 100, not only dots, no leading dash (the
 *  command would read it as an option). */
export function repoNameProblem(input: string): string | null {
  const name = input.trim();
  if (!name) return 'Type a name for the repo.';
  if (name.length > 100) return 'Keep it to 100 characters.';
  if (!/^[A-Za-z0-9._-]+$/.test(name)) return 'Letters, digits, dots, dashes and underscores only.';
  if (/^\.+$/.test(name)) return 'A name can’t be only dots.';
  if (name.startsWith('-')) return 'Start with a letter, a digit, a dot or an underscore.';
  return null;
}

export interface SetupCard {
  /** The trimmed name, or the problem with it. */
  name: string;
  problem: string | null;
  /** The one command the person runs; null while the name has a problem. */
  command: string | null;
  /** The SSH address to connect, with the placeholder when the login is unknown. */
  url: string | null;
  /** The address for the Connect field: only with a known login. */
  prefill: string | null;
}

/** The card's command and address for a repo name and an owner (a GitHub
 *  login, or null when Hollow doesn't know it). Without a login the command
 *  names no owner, so gh creates the repo under the account it is signed in
 *  as and the line can be run as copied. Private, no README: the first sync
 *  fills it. */
export function setupCard(nameInput: string, ownerInput: string | null): SetupCard {
  const name = nameInput.trim();
  const problem = repoNameProblem(name);
  if (problem) return { name, problem, command: null, url: null, prefill: null };
  const owner = githubLogin(ownerInput);
  const url = `git@github.com:${owner ?? SETUP_OWNER_PLACEHOLDER}/${name}.git`;
  return { name, problem, command: `gh repo create ${owner ? `${owner}/` : ''}${name} --private`, url, prefill: owner ? url : null };
}

/** The Connect field after the card's address changes: the card fills it
 *  only while it is empty or still holds the card's own last address, so
 *  what the person typed there is never replaced. */
export function prefillRemote(current: string, lastPrefill: string | null, next: string | null): string {
  if (!next) return current.trim() && current !== lastPrefill ? current : '';
  return !current.trim() || current === lastPrefill ? next : current;
}
// CX22: the portable setup for people without Hollow. The brief lives in
// src/contextSetupBrief.ts and is loaded lazily when copied on demand.
export const GUIDE_URL = 'https://github.com/example-org/org-context/blob/main/GUIDE.md';
export const BRIEF_COPIED = 'Setup brief copied. Paste it into any AI assistant and say: Follow this brief and set up my context.';

// ── Context check (CX10): a local pre-push hook per repo ──

/** A repo's context check pill. */
export function checkPill(repo: ContextCheckRepo): Pill {
  switch (repo.state) {
    case 'installed': return { text: 'Checked before push', state: 'ok' };
    case 'not_installed': return { text: 'Not checked' };
    case 'other_hook': return { text: 'Has its own hook', state: 'warn' };
    default: return { text: 'Unavailable', state: 'warn' };
  }
}

/** The button a repo row offers, or none (another hook, or no hook folder). */
export function checkButton(repo: ContextCheckRepo): 'Install context check' | 'Remove' | null {
  if (repo.state === 'installed') return 'Remove';
  return repo.state === 'not_installed' ? 'Install context check' : null;
}

/** A row button's accessible name: "Install context check for <repo>" or
 *  "Remove context check for <repo>". */
export function checkButtonName(label: 'Install context check' | 'Remove', repo: string): string {
  return label === 'Remove' ? `Remove context check for ${repo}` : `Install context check for ${repo}`;
}

/** The write-time mode in one line (`context.leakChecks`). */
export function leakModeLine(status: ContextCheckStatus | undefined, error = ''): string {
  if (error) return `Couldn’t read the context checks: ${error}`;
  if (!status) return 'Checking…';
  return status.mode === 'block'
    ? 'Leak checks block: a project doc write that names a computer or a person is refused.'
    : 'Leak checks flag only: such a write is saved and recorded for review. A secret is always refused.';
}

/** The built-in "Monthly context audit" automation template: the person adds
 *  it from Automations, + New automation; nothing turns it on by itself. */
export const CONTEXT_AUDIT_TEMPLATE = {
  id: 'context-audit',
  label: 'Monthly context audit',
  description: 'Once a month, check every context and project repo for secrets and misplaced facts.',
  schedule: '0 0 9 1 * * *',
  prompt: 'Run the Hollow app tool context.audit (no arguments), then context.leak_flags with days 31. Write outputs/context-audit-<year>-<month>.md: for each repo and layer the block and warn counts and each finding as file, line, rule id and severity; any repos the audit skipped and how many findings it left out; then the leak flags by rule, layer and source, and which blocking findings need a fix and in which layer the text belongs. Never copy a matched value or a line of file text into the note or anywhere else: file, line and rule id only. Then call notifications.create_notification with title "Monthly context audit", body one line giving the block and warn totals and the note’s path, and kind "info". Report in Hollow; do not create a tracker issue. Change nothing in the repos.',
} as const;

// ── Connect this Mac's context (F3) ──────────────────────────────────────

/** A row's name on the card. */
export const MAC_LAYER_LABEL: Record<MacContextLayer['layer'], string> = {
  personal: 'Personal context', org: 'Organization context (read only)', home: 'This Mac’s name and home file', vault: 'Vault',
};

/** The card's heading line: which Mac brought it, or none. */
export function macContextLead(offer: MacContextOffer): string {
  if (!offer.otherName) return 'Clone your context repos onto this Mac and name it.';
  const what = offer.otherKind === 'runner' ? 'runner' : 'Mac';
  return `This Mac is connected to the ${what} ${offer.otherName}. Clone your context here and name this Mac.`;
}

/** Rows ticked at first: every missing one. Connected and held rows can't be ticked. */
export const macTickable = (row: MacContextLayer) => row.state === 'missing';
export const macTickedAtFirst = (offer: MacContextOffer) => offer.layers.filter(macTickable).map(r => r.layer);

/** Whether a ticked row still needs the repo address field. */
export const macNeedsUrl = (row: MacContextLayer) => (row.layer === 'personal' || row.layer === 'org') && row.state === 'missing';

/** The request Connect sends: ticked rows and their addresses (typed, else known); never a path. */
export function macConnectRequest(offer: MacContextOffer, ticked: readonly string[], urls: { personal: string; org: string }, slug: string): MacContextRequest {
  const known = (layer: 'personal' | 'org') => urls[layer].trim() || offer.layers.find(r => r.layer === layer)?.url || null;
  return {
    pair: offer.pair,
    layers: offer.layers.filter(r => macTickable(r) && ticked.includes(r.layer)).map(r => r.layer),
    personalUrl: ticked.includes('personal') ? known('personal') : null,
    orgUrl: ticked.includes('org') ? known('org') : null,
    slug: slug.trim() || null,
  };
}

/** Why Connect can't run yet, or null. */
export function macConnectProblem(offer: MacContextOffer, req: MacContextRequest): string | null {
  if (!req.layers.length) return 'Tick at least one row.';
  for (const layer of ['personal', 'org'] as const) {
    if (!req.layers.includes(layer)) continue;
    const url = layer === 'personal' ? req.personalUrl : req.orgUrl;
    if (!url) return `Paste the ${layer === 'personal' ? 'personal' : 'organization'} repo’s address.`;
    const why = remoteProblem(url);
    if (why) return why;
  }
  if (req.layers.includes('home') && !offer.slug && !req.slug && !offer.suggestedSlug) return 'Type a name for this Mac.';
  if (req.slug) return slugProblem(req.slug);
  return null;
}

/** The pill for one step's result. */
export function macStepPill(state: MacContextReport['steps'][number]['state']): Pill {
  switch (state) {
    case 'done': return { text: 'Done', state: 'ok' };
    case 'skipped': return { text: 'Already here', state: 'ok' };
    case 'held': return { text: 'Held' };
    case 'failed': return { text: 'Failed', state: 'error' };
    case 'refused': return { text: 'Refused', state: 'error' };
  }
}

// ── The first sync's dry run (PX1) ───────────────────────────────────────

/** Names listed per category before "and N more". */
export const SEED_NAMES_SHOWN = 6;

export interface SeedLine {
  /** `learnings/`, `memory/`... */
  folder: string;
  /** What the first sync would do with these files, in words. */
  what: string;
  /** The file names, at most [`SEED_NAMES_SHOWN`], then "and N more". */
  names: string;
}

const nameList = (names: readonly string[]): string =>
  names.length > SEED_NAMES_SHOWN ? `${names.slice(0, SEED_NAMES_SHOWN).join(', ')} and ${names.length - SEED_NAMES_SHOWN} more` : names.join(', ');

/** One line per folder and kind of change, folders in sync order, nothing for
 *  a folder whose two sides match. The report is the backend's dry run of the
 *  personal sync's first cycle; this only words it. */
export function seedLines(folders: readonly ContextSeedFolder[]): SeedLine[] {
  const out: SeedLine[] = [];
  for (const f of folders) {
    const kinds: [string[], string][] = [
      [f.onlyVault, 'only in the vault: the first sync sends it to the repo'],
      [f.onlyClone, 'only in the repo: the first sync writes it into the vault'],
      [f.differ, 'different in each: the vault’s stays and the repo’s is saved beside it (a clash card)'],
    ];
    for (const [names, what] of kinds) {
      if (names.length) out.push({ folder: `${f.folder}/`, what: `${names.length} ${what}`, names: nameList(names) });
    }
  }
  return out;
}

/** Whether the dry run has anything to show worth a fold: some file differs between the sides. */
export const seedHasChanges = (folders: readonly ContextSeedFolder[]): boolean => seedLines(folders).length > 0;
