// Browser-mock Settings, Context (CX17): `context_layers_status`, the sync
// engine's `context_sync_*` and `configure_personal_sync`, Local only, the
// org access check, the clone folders, the machine name and the context
// check rows (CX10: install and remove flip the state), answered from
// one in-memory fixture under /Users/mock. Nothing on disk is touched.
//
// Knobs (localStorage):
// - `mockContextLayers=connected`: the personal row starts with a remote and
//   the org clone is set (default: nothing set up yet).
// - `mockContextOrgAccess`: the access check's answer (`ok` by default;
//   `denied`, `offline`).
// - `mockMacContext=offer`: "Connect this Mac's context" (F3) is open for
//   the runner `mock-runner` (default: no other Mac, the fold only).
import type { MacContextLayer, MacContextOffer, MacContextStep } from './contextLayersApi';
import type { ContextCheckStatus, ContextLayersStatus, ContextRootStatus, ContextSyncStatus, OrgAccessCheck } from './contextLayersApi';

const HOME = '/Users/mock';
const get = (k: string) => localStorage.getItem(k);

function root(layer: 'personal' | 'org', path: string | null): ContextRootStatus {
  return {
    layer, path: path ?? `${HOME}/Hollow/Context/${layer}`, configured: !!path, exists: !!path, gitClone: !!path,
    lastCommitAt: path ? '2026-11-14T21:10:00Z' : null, problem: null,
  };
}

function initial(): ContextLayersStatus {
  const connected = get('mockContextLayers') === 'connected';
  return {
    home: { slug: connected ? 'mock-macbook' : null, suggestedSlug: 'mock-macbook', file: connected ? 'home/mock-macbook.md' : null, fileExists: connected, root: root('personal', connected ? `${HOME}/Hollow/Context/personal` : null) },
    personal: {
      root: 'personal', state: connected ? 'idle' : 'off', enabled: connected, remote: connected ? 'git@github.com:mock/personal-context.git' : null,
      pendingCommits: 0, unsyncedFiles: 0, text: connected ? 'Up to date' : 'Not set up', lastSyncAt: connected ? '2026-11-15T09:12:00Z' : null,
      lastError: null, blockedBy: null, clashCopies: [],
    },
    org: { root: root('org', connected ? `${HOME}/Hollow/Context/org` : null), remote: connected ? 'git@github.com:mock-org/org-context.git' : null, lastPullAt: connected ? '2026-11-15T08:40:00Z' : null },
    projects: {
      withContext: [
        { title: 'hollow', path: `${HOME}/Hollow/Projects/Personal/hollow`, files: ['CLAUDE.md', 'AGENTS.md'], git: true },
        { title: 'pricing-study', path: `${HOME}/Hollow/Projects/pricing-study`, files: ['CLAUDE.md'], git: true },
      ],
      total: 5, capped: false, error: null,
    },
    vault: { setUp: false, enabled: false, remote: null, text: 'Not set up' },
  };
}

let state: ContextLayersStatus | undefined;
const current = () => (state ??= initial());

// Context check (CX10): one project repo per fixture folder, nothing installed.
let checks: ContextCheckStatus | undefined;
const currentChecks = () => (checks ??= {
  mode: 'flag',
  repos: current().projects.withContext.filter(p => p.git).map(p => ({ title: p.title, path: p.path, layer: 'project' as const, state: 'not_installed' as const, problem: null })),
});

function personal(next: Partial<ContextSyncStatus>): ContextSyncStatus {
  const s = current();
  s.personal = { ...s.personal, ...next };
  return s.personal;
}

const COMMANDS = new Set([
  'context_layers_status', 'context_sync_status', 'context_sync_now', 'set_context_sync_enabled', 'configure_personal_sync',
  'context_personal_local_only', 'context_org_access_check', 'set_context_root', 'set_context_machine_slug',
  'context_check_status', 'context_install_check', 'context_remove_check',
  'mac_context_offer', 'mac_context_answer', 'mac_context_connect',
]);

let macAnswered = false;

function macOffer(s: ContextLayersStatus): MacContextOffer {
  const row = (layer: MacContextLayer['layer'], here: boolean, text: string): MacContextLayer =>
    ({ layer, state: layer === 'vault' ? 'held' : here ? 'connected' : 'missing', text, target: layer === 'personal' || layer === 'org' ? `${HOME}/Hollow/Context/${layer}` : null, url: null });
  const layers = [
    row('personal', s.home.root.gitClone, s.home.root.gitClone ? 'Personal context is cloned.' : `Clone the personal repo into ${HOME}/Hollow/Context/personal.`),
    row('org', s.org.root.gitClone, s.org.root.gitClone ? 'Organization context is cloned.' : `Clone the organization repo into ${HOME}/Hollow/Context/org.`),
    row('home', !!s.home.slug && s.home.fileExists, s.home.slug ? `This Mac is ${s.home.slug}.` : 'Name this Mac and write its home file in the personal clone.'),
    row('vault', false, 'The vault has no remote yet (its first push is held), so this Mac keeps its own vault.'),
  ];
  const other = get('mockMacContext') === 'offer';
  const missing = layers.some(r => r.state === 'missing');
  return {
    pair: other ? 'm-mock+runner:mock-runner' : null, otherName: other ? 'mock-runner' : null, otherKind: other ? 'runner' : null,
    open: other && missing && !macAnswered, layers, slug: s.home.slug, suggestedSlug: s.home.slug ? null : s.home.suggestedSlug,
  };
}

export const isMockContextLayersCommand = (command: string) => COMMANDS.has(command);

/** For tests: start again from the knobs. */
export function resetMockContextLayers(): void { state = undefined; checks = undefined; macAnswered = false; }

export function mockContextLayersCommand(command: string, args: Record<string, unknown>): unknown {
  const s = current();
  switch (command) {
    case 'context_layers_status': return structuredClone(s);
    case 'context_sync_status': return { ...s.personal };
    case 'context_sync_now':
      if (!s.personal.remote) throw new Error('Connect a remote first.');
      return personal({ state: 'idle', text: 'Up to date', lastSyncAt: new Date().toISOString() });
    case 'set_context_sync_enabled': {
      if (!s.personal.remote) throw new Error('Connect a remote first.');
      const enabled = args.enabled === true;
      return personal({ enabled, state: enabled ? 'idle' : 'off', text: enabled ? 'Up to date' : 'Sync is off' });
    }
    case 'configure_personal_sync': {
      const remote = String(args.remote ?? '').trim();
      if (/^https?:\/\/[^/]*@/.test(remote)) throw new Error('That remote can’t be used: it carries credentials; use a credential helper or an SSH key.');
      return personal({ remote, enabled: true, state: 'idle', text: 'Up to date' });
    }
    case 'context_personal_local_only': return personal({ remote: null, enabled: false, state: 'off', text: 'Sync is off' });
    case 'context_org_access_check': {
      if (!s.org.root.configured) return { state: 'missing', text: 'No org clone on this Mac yet. Choose its folder first.' } satisfies OrgAccessCheck;
      const want = get('mockContextOrgAccess');
      if (want === 'denied') return { state: 'denied', text: 'No access to the org repo from this Mac. Ask its owner to add you.' } satisfies OrgAccessCheck;
      if (want === 'offline') return { state: 'offline', text: 'Couldn’t reach the org repo’s host. Try again online.' } satisfies OrgAccessCheck;
      return { state: 'ok', text: 'This Mac can fetch the org repo.' } satisfies OrgAccessCheck;
    }
    case 'set_context_root': {
      const layer = args.layer === 'org' ? 'org' : 'personal';
      const path = typeof args.path === 'string' && args.path.trim() ? args.path.trim() : null;
      const next = root(layer, path);
      if (layer === 'org') s.org = { ...s.org, root: next, lastPullAt: null };
      else s.home = { ...s.home, root: next, fileExists: false };
      return next;
    }
    case 'context_check_status': return structuredClone(currentChecks());
    case 'context_install_check':
    case 'context_remove_check': {
      const repo = currentChecks().repos.find(r => r.path === args.path);
      if (!repo) throw new Error('That folder isn’t one of Hollow’s context or project repos (Settings, Context lists them). Nothing changed.');
      const install = command === 'context_install_check';
      repo.state = install ? 'installed' : 'not_installed';
      return { applied: true, text: install ? `Installed: pushes from this repo now stop on a blocking finding (${repo.layer} layer).` : 'Removed: pushes from this repo are no longer checked.' };
    }
    case 'mac_context_offer': {
      const offer = macOffer(s);
      if (!offer.open && args.any !== true) return null;
      return offer;
    }
    case 'mac_context_answer': macAnswered = true; return null;
    case 'mac_context_connect': {
      const req = args.request as { layers: string[]; personalUrl: string | null; orgUrl: string | null; slug: string | null; pair: string | null };
      const steps: MacContextStep[] = req.layers.map(layer => {
        if (layer === 'personal' || layer === 'org') {
          const path = `${HOME}/Hollow/Context/${layer}`;
          if (layer === 'org') s.org = { ...s.org, root: root('org', path), remote: req.orgUrl };
          else s.home = { ...s.home, root: root('personal', path) };
          return { layer, state: 'done', text: `Cloned ${layer === 'org' ? req.orgUrl : req.personalUrl} into ${path}.` };
        }
        if (layer === 'home') {
          const slug = s.home.slug ?? req.slug ?? s.home.suggestedSlug ?? 'mock-mac';
          s.home = { ...s.home, slug, file: `home/${slug}.md`, fileExists: true };
          return { layer, state: 'done', text: `This Mac is ${slug}; wrote home/${slug}.md in the personal clone.` };
        }
        return { layer, state: 'held', text: 'Not cloned: the vault has no remote until its first push (V6). This Mac keeps its own vault.' };
      });
      const connected = steps.every(x => x.state !== 'failed');
      if (connected) macAnswered = true;
      return { steps, connected };
    }
    case 'set_context_machine_slug': {
      const slug = typeof args.slug === 'string' && args.slug.trim() ? args.slug.trim() : null;
      s.home = { ...s.home, slug, file: slug ? `home/${slug}.md` : null, fileExists: false };
      return slug;
    }
  }
  throw new Error(`mock: unknown context command ${command}`);
}
