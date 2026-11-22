// Browser-mock Disk space (overnight D1) and its two cleanups (overnight
// H20): `disk_guard`, `install_targets` / `prune_install_targets` and
// `worktree_hygiene` / `clean_worktrees`. What has been cleaned is kept in
// localStorage (`mockDiskPruned`, `mockDiskCleaned`), so the next measure
// shows the folders gone and more space free. Each delete is recorded in
// `mockDiskCleanups` ({ command, expectedHash }) for the smoke test, and a
// hash that isn't the current preview's refuses with the backend's words
// (Rust `install_targets::prune_in`, `worktree_hygiene::clean_for`; the
// panel rewords them, diskGuard.ts `cleanupErrorText`), deleting nothing.
// Setting `mockDiskPruned` / `mockDiskCleaned` while a dialog is open makes
// its list stale. Nothing on disk is touched: these are fixtures.
//
// Repo features R7b: `hollow_databases` (Settings, Advanced, Hollow's
// databases) and Disk space's third cleanup, `stale_indexes` /
// `remove_stale_indexes` (Remove old indexes…), the same way:
// `mockDiskIndexesRemoved` keeps the removal, deletes are recorded in
// `mockDiskCleanups`, and a stale hash refuses. `mockDbUnreadable=1` makes
// Check integrity unable to open the companion ledger (A4), as the backend
// reports it: `error` set, no `quickCheck`.

import type { DiskReport, HollowDatabases, HollowDbFile, InstallTargetsPlan, InstallTargetsPruned, StaleIndexesPlan, StaleIndexesRemoved, WorktreeHygieneReport, WorktreesCleaned } from './types';

const GB = 1024 ** 3;
const CACHES = '/Users/mock/Library/Caches';
const OLD_TARGETS = [`${CACHES}/hollow-install-target-old-lane`, `${CACHES}/hollow-install-target-spike`];
const WORKTREE = '/Users/mock/Hollow/Worktrees/Hollow/finished-lane';
const TILDE = '/Users/mock/Hollow/Worktrees/Hollow/finished-lane/src-tauri/~';
const pruned = () => localStorage.getItem('mockDiskPruned') === '1';
const cleaned = () => localStorage.getItem('mockDiskCleaned') === '1';
/** `mockDiskInUseUnknown=1`: as if lsof failed, so every worktree and "~"
 *  folder stays (the backend's `inUseUnknown`). */
const inUseUnknown = () => localStorage.getItem('mockDiskInUseUnknown') === '1';

function record(command: string, expectedHash: unknown): void {
  const log = JSON.parse(localStorage.getItem('mockDiskCleanups') ?? '[]') as unknown[];
  localStorage.setItem('mockDiskCleanups', JSON.stringify([...log, { command, expectedHash }]));
}

export function mockDiskGuard(): DiskReport {
  const threshold = Number(localStorage.getItem('mockDiskThreshold') ?? 50);
  const free = 120 + (pruned() ? 20 : 0) + (cleaned() ? 6 : 0);
  const folders: DiskReport['folders'] = [
    { path: `${CACHES}/hollow-install-target-main`, kind: 'cargo-target', bytes: 14 * GB },
    ...(pruned() ? [] : [
      { path: OLD_TARGETS[0], kind: 'cargo-target' as const, bytes: 12 * GB },
      { path: OLD_TARGETS[1], kind: 'cargo-target' as const, bytes: 8 * GB },
    ]),
    ...(cleaned() ? [] : [{ path: WORKTREE, kind: 'worktree' as const, bytes: 6 * GB }]),
  ];
  return {
    freeBytes: free * GB, thresholdGb: threshold, low: free < threshold, vault: null, folders,
    cleanup: [
      'Install build folders: ask an agent for build.install_targets, then build.prune_install_targets (asks first; keeps the installed and rollback builds).',
      'Worktrees: ask an agent for build.worktree_hygiene, then build.clean_worktrees (asks first; only merged, clean, pushed ones).',
    ],
  };
}

export function mockInstallTargets(): InstallTargetsPlan {
  const kept = [
    { path: `${CACHES}/hollow-install-target-main`, name: 'hollow-install-target-main', lastUsed: '2026-10-14T21:10:00Z', keep: 'built the installed app' },
    { path: `${CACHES}/hollow-install-target-rollback`, name: 'hollow-install-target-rollback', lastUsed: '2026-10-12T09:00:00Z', keep: 'built the rollback copy' },
  ];
  const old = pruned() ? [] : [
    { path: OLD_TARGETS[0], name: 'hollow-install-target-old-lane', lastUsed: '2026-10-03T18:00:00Z', keep: null },
    { path: OLD_TARGETS[1], name: 'hollow-install-target-spike', lastUsed: '2026-09-28T11:30:00Z', keep: null },
  ];
  return { targets: [...kept, ...old], remove: old.map(t => t.path), keepNewest: 2, hash: pruned() ? 'mock-targets-empty' : 'mock-targets-2', notes: [], leftovers: [] };
}

export function mockPruneInstallTargets(args?: Record<string, unknown>): InstallTargetsPruned {
  record('prune_install_targets', args?.expectedHash);
  const plan = mockInstallTargets();
  if (args?.expectedHash !== plan.hash) throw 'The folders to delete changed since the card was shown. Nothing was deleted; ask again to see the new list.';
  localStorage.setItem('mockDiskPruned', '1');
  return { removed: plan.remove, failed: [], kept: plan.targets.length - plan.remove.length };
}

export function mockWorktreeHygiene(): WorktreeHygieneReport {
  const unknown = inUseUnknown();
  const done = cleaned();
  const goes = !done && !unknown;
  return {
    repos: [{
      repo: '/Users/mock/Projects/Hollow', defaultBranch: 'origin/main',
      worktrees: [
        ...(done ? [] : [{ path: WORKTREE, branch: 'n15/finished-lane', workspace: 'build-finished-lane', merged: true, clean: true, unpushed: 0,
          ignored: ['node_modules', 'dist'], ignoredCount: 2, inUse: false, removable: !unknown,
          reason: unknown ? "couldn't read what is running, so nothing is removed" : null }]),
        { path: '/Users/mock/Hollow/Worktrees/Hollow/open-lane', branch: 'n15/open-lane', workspace: null, merged: false, clean: false, unpushed: 3,
          ignored: [], ignoredCount: 0, inUse: true, removable: false, reason: 'not merged' },
      ],
      localOnlyBranches: [{ name: 'spike/notes', commits: 2, lastCommit: '2026-10-10T12:00:00Z' }],
      tildeFolders: goes ? [{ path: TILDE, bytes: Math.round(1.5 * GB) }] : [],
      error: null,
    }],
    remove: goes ? [WORKTREE, TILDE] : [],
    hash: goes ? 'mock-worktrees-2' : 'mock-worktrees-empty',
    tildeBytes: goes ? Math.round(1.5 * GB) : 0,
    inUseUnknown: unknown,
  };
}

export function mockCleanWorktrees(args?: Record<string, unknown>): WorktreesCleaned {
  record('clean_worktrees', args?.expectedHash);
  const r = mockWorktreeHygiene();
  if (args?.expectedHash !== r.hash) throw 'The worktrees and folders to remove changed since the card was shown. Nothing was removed; ask again to see the new list.';
  localStorage.setItem('mockDiskCleaned', '1');
  return { removed: r.remove, failed: [] };
}

const MB = 1024 ** 2;
const SUPPORT = '/Users/mock/Library/Application Support/Hollow';
const indexesRemoved = () => localStorage.getItem('mockDiskIndexesRemoved') === '1';
type Db = Pick<HollowDbFile, 'kind' | 'stem' | 'owner' | 'ownerName' | 'bytes' | 'walBytes' | 'schemaVersion'> & { rows: Array<[string, number]> };
const FOLDER: Record<HollowDbFile['kind'], string> = { sessions: 'Sessions', searchIndex: 'SearchIndex', embedIndex: 'EmbedIndex', companion: 'Companion', projectIndex: 'SearchIndex' };
const THIS = 'hollow-vault-1d1d4c39cd10e040';
const OTHER = 'Second-brain-20f08a1b2c3d4e5f';
const STALE: Db[] = [
  { kind: 'searchIndex', stem: OTHER, owner: 'otherVault', ownerName: 'Second-brain', bytes: 120 * MB, walBytes: 0, schemaVersion: 2, rows: [['notes', 2_400]] },
  { kind: 'searchIndex', stem: 'Vault-3a14c0ffee123456', owner: 'otherVault', ownerName: 'Vault', bytes: 60 * MB, walBytes: 0, schemaVersion: 1, rows: [['notes', 900]] },
  { kind: 'embedIndex', stem: OTHER, owner: 'otherVault', ownerName: 'Second-brain', bytes: 90 * MB, walBytes: 0, schemaVersion: 3, rows: [['chunks', 18_000], ['vectors', 18_000]] },
];
const dbPath = (d: Pick<Db, 'kind' | 'stem'>) => `${SUPPORT}/${FOLDER[d.kind]}/${d.stem}.db`;

export function mockHollowDatabases(args?: Record<string, unknown>): HollowDatabases {
  const check = args?.check === true;
  const dbs: Db[] = [
    { kind: 'sessions', stem: THIS, owner: 'thisVault', ownerName: null, bytes: 439 * MB, walBytes: 0, schemaVersion: 0, rows: [['sessions', 3_120], ['turns', 41_000]] },
    { kind: 'sessions', stem: OTHER, owner: 'otherVault', ownerName: 'Second-brain', bytes: Math.round(0.3 * MB), walBytes: 0, schemaVersion: 0, rows: [['sessions', 12], ['turns', 80]] },
    { kind: 'searchIndex', stem: THIS, owner: 'thisVault', ownerName: null, bytes: 86 * MB, walBytes: 16 * MB, schemaVersion: 2, rows: [['notes', 5_200]] },
    { kind: 'embedIndex', stem: THIS, owner: 'thisVault', ownerName: null, bytes: 72 * MB, walBytes: 0, schemaVersion: 3, rows: [['chunks', 31_000], ['vectors', 31_000]] },
    { kind: 'companion', stem: 'applied', owner: 'app', ownerName: null, bytes: 40 * 1024, walBytes: 0, schemaVersion: 0, rows: [['applied', 14]] },
    ...(indexesRemoved() ? [] : STALE),
  ];
  const kinds = Object.keys(FOLDER);
  dbs.sort((a, b) => kinds.indexOf(a.kind) - kinds.indexOf(b.kind));
  const stale = mockStaleIndexes();
  return {
    folder: SUPPORT,
    files: dbs.map(({ rows, ...d }) => {
      const unreadable = check && d.kind === 'companion' && localStorage.getItem('mockDbUnreadable') === '1';
      return {
        ...d, path: dbPath(d), modified: '2026-10-18T09:00:00-04:00',
        quickCheck: check && !unreadable ? 'ok' : null,
        tables: check && !unreadable ? rows.map(([name, n]) => ({ name, rows: n })) : null,
        error: unreadable ? 'unable to open database file' : null,
      };
    }),
    staleCount: stale.files.length, staleBytes: stale.bytes,
  };
}

export function mockStaleIndexes(): StaleIndexesPlan {
  const files = indexesRemoved() ? [] : STALE.map(d => ({ path: dbPath(d), kind: d.kind, ownerName: d.ownerName ?? '', bytes: d.bytes, modified: '2026-09-30T18:00:00-04:00', why: null }));
  return {
    folder: SUPPORT, files, bytes: files.reduce((n, f) => n + f.bytes, 0),
    hash: files.length ? 'mock-indexes-3' : 'mock-indexes-empty',
    kept: [{ path: dbPath({ kind: 'sessions', stem: OTHER }), kind: 'sessions', ownerName: 'Second-brain', why: "another vault's conversations: never removed" }],
  };
}

export function mockRemoveStaleIndexes(args?: Record<string, unknown>): StaleIndexesRemoved {
  record('remove_stale_indexes', args?.expectedHash);
  const plan = mockStaleIndexes();
  if (args?.expectedHash !== plan.hash) throw 'The old indexes to remove changed since the card was shown. Nothing was removed; ask again to see the new list.';
  localStorage.setItem('mockDiskIndexesRemoved', '1');
  return { removed: plan.files.map(f => f.path), failed: [], bytes: plan.bytes };
}
