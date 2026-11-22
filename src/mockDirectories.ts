/**
 * Browser-mock connected-project inventory (next-level P07). The default
 * fixture is one project, Launch, with 2,804 listed files: a 2,800-file
 * archive, a PDF (read by Hollow: it opens to its text, R14a), a 3.4 MB CSV
 * export (too large to read)
 * and `Launch/leases/garage-14-lease-renewal-terms.md`, which sits after item
 * 2,500 in path order: the programme's acceptance fixture (before P07 the
 * real scan never listed it). Tests and reviews can still seed their own
 * `mockDirectories` in localStorage, with or without a `files` list.
 */
import type { ConnectedFile, ConnectedFilePage, ConnectedFileState, ConnectedIndexStatus } from './types';

const ROOT = '/Hollow/Projects';
export const LEASE_PATH = `${ROOT}/Launch/leases/garage-14-lease-renewal-terms.md`;
export const DECK_PATH = `${ROOT}/Launch/decks/q3-review.pdf`;
const DAY = 86_400_000;
const BASE = Date.parse('2026-07-15T13:00:00-04:00');

function file(relative: string, size: number, state: ConnectedFileState, modified: number): ConnectedFile {
  return { path: `${ROOT}/${relative}`, relative, projectId: 'local-demo', size, readable: state === 'text', state, modified };
}

let generated: ConnectedFile[] | null = null;
/** The default Launch inventory, in path order (generated once). */
export function defaultFiles(): ConnectedFile[] {
  if (generated) return generated;
  const files: ConnectedFile[] = [file('Launch/README.md', 64, 'text', BASE)];
  for (let i = 1; i <= 2_800; i++) files.push(file(`Launch/archive/2024/visit-${String(i).padStart(4, '0')}.md`, 120, 'text', BASE - (2_801 - i) * 3_600_000));
  files.push(file('Launch/decks/q3-review.pdf', 2_400_000, 'extracted', BASE - 2 * DAY));
  files.push(file('Launch/exports/ledger-2025.csv', 3_400_000, 'tooLarge', BASE - 3 * DAY));
  files.push(file('Launch/leases/garage-14-lease-renewal-terms.md', 940, 'text', BASE - DAY));
  return (generated = files);
}

const normalize = (f: Partial<ConnectedFile> & { path: string; relative: string; projectId: string; size: number; readable: boolean }): ConnectedFile =>
  ({ ...f, state: f.state ?? (f.readable ? 'text' : 'needsReader'), modified: f.modified ?? 0 });

function defaultCoverage(now: string) {
  return { projects: [{ id: 'local-demo', title: 'Launch', indexed: 0, found: 0, capped: false, folders: [], omitted: 0, tooLarge: 0, needsReader: 0, extracted: 0 }], scanLimited: false, maxFiles: 200_000, maxProjectFiles: 50_000, excluded: [] as string[], indexed: 0, available: 0, omitted: 0, tooLarge: 0, needsReader: 0, extracted: 0, inventoryLimited: false, changedAt: now };
}

/** The mock connection with its full file list (never returned whole). */
export function mockDirectoryState(now: string): any {
  const stored = JSON.parse(localStorage.getItem('mockDirectories') ?? 'null');
  const base = stored ?? (new URLSearchParams(location.search).get('setup') === '1'
    ? { version: 0, pendingRestart: false, homePath: '', projectsPath: '', vaultPath: '', refreshedAt: '', projects: [], files: [], warnings: [] }
    : { version: 1, pendingRestart: false, homePath: '/Hollow', projectsPath: ROOT, vaultPath: '/Hollow/Vault', refreshedAt: now, projects: [{ id: 'local-demo', title: 'Launch', path: `${ROOT}/Launch` }], warnings: [], coverage: defaultCoverage(now) });
  const excluded: string[] = base.coverage?.excluded ?? [];
  const files = ((base.files ?? defaultFiles()) as ConnectedFile[]).map(normalize).filter(f => !excluded.some(r => f.relative.startsWith(`${r}/`)));
  return { ...base, files };
}

/** `directories_status` / `refresh_directories`: the state without its file
 *  list, with P07 totals recounted for a P07-shaped coverage. */
export function mockDirectoryStatus(state: any, refreshedAt?: string): any {
  const { files, ...rest } = state;
  const out = { ...rest, ...(refreshedAt && rest.refreshedAt ? { refreshedAt } : {}) };
  if (out.coverage && out.coverage.indexed !== undefined) {
    const list = files as ConnectedFile[];
    const projects = out.coverage.projects.map((p: any) => {
      const mine = list.filter(f => f.projectId === p.id);
      const folders = new Map<string, number>();
      for (const f of mine) { const parts = f.relative.split('/'); if (parts.length > 2) folders.set(`${parts[0]}/${parts[1]}`, (folders.get(`${parts[0]}/${parts[1]}`) ?? 0) + 1); }
      return { ...p, indexed: mine.length, found: mine.length, omitted: 0, capped: false, tooLarge: mine.filter(f => f.state === 'tooLarge').length, needsReader: mine.filter(f => f.state === 'needsReader').length, extracted: mine.filter(f => f.state === 'extracted').length,
        folders: [...folders].map(([relative, n]) => ({ relative, files: n })).sort((a, b) => b.files - a.files).slice(0, 8) };
    });
    out.coverage = { ...out.coverage, projects, indexed: list.length, available: list.length, omitted: 0, tooLarge: list.filter(f => f.state === 'tooLarge').length, needsReader: list.filter(f => f.state === 'needsReader').length, extracted: list.filter(f => f.state === 'extracted').length };
  }
  return out;
}

/** `connected_index_status` (repo features R14b): every listed text, PDF
 *  and Office file indexed once `mockContentIndexOn` is set (off by default). */
export function mockContentIndexStatus(state: any): ConnectedIndexStatus {
  const enabled = localStorage.getItem('mockContentIndexOn') === '1';
  const total = (state.files as ConnectedFile[]).filter(f => f.state === 'text' || f.state === 'extracted').length;
  return { enabled, indexed: enabled ? total : 0, total, failed: 0, imageOnly: 0, skipped: 0 };
}

/** `connected_files`: filter, rank (name hits first) and page like the backend. */
export function mockConnectedFiles(state: any, args: { query?: string; projectId?: string; fileState?: ConnectedFileState; offset?: number; limit?: number } = {}): ConnectedFilePage {
  const files = state.files as ConnectedFile[];
  const terms = (args.query ?? '').toLowerCase().split(/\s+/).filter(Boolean).slice(0, 8);
  const name = (f: ConnectedFile) => (f.relative.split('/').pop() ?? '').toLowerCase();
  let hits = files.filter(f => (!args.projectId || f.projectId === args.projectId) && (!args.fileState || f.state === args.fileState) && terms.every(t => f.relative.toLowerCase().includes(t)));
  if (terms.length) hits = hits.map((f, i) => ({ f, i, n: terms.filter(t => name(f).includes(t)).length })).sort((a, b) => b.n - a.n || a.i - b.i).map(x => x.f);
  const offset = Math.max(0, args.offset ?? 0), limit = Math.min(500, Math.max(1, args.limit ?? 100));
  return { total: hits.length, offset, files: hits.slice(offset, offset + limit), indexed: files.length, omitted: state.coverage?.omitted ?? 0, direct: [], refreshedAt: state.refreshedAt ?? '' };
}

/** `read_connected_file`: the lease fixture's text, honest refusals for unread files. */
export function mockReadConnectedFile(state: any, path: string): string {
  const f = (state.files as ConnectedFile[]).find(x => x.path === path);
  if (f?.state === 'tooLarge') throw new Error('Not read: the file is larger than the 1 MB text reader limit. Open the original, or export a smaller text file.');
  if (f?.state === 'needsReader') throw new Error('This source requires a native reader or text export');
  if (path === DECK_PATH) return '## Page 1\nQ3 regional review\nOccupancy rose to 87% across the Launch garages.\n\n## Page 2\nNext: price the Garage 14 renewal before 1 March.\n';
  if (path === LEASE_PATH) return '# Garage 14 lease renewal terms\n\nTerm: five years from 1 March 2027.\nRent: a 3% annual escalator on the current base.\nNotice: either side, 180 days before expiry.\n';
  if (f && f.relative.startsWith('Launch/archive/')) return `# Site visit\n\nRoutine site visit (${f.relative.split('/').pop()}). Nothing to report.\n`;
  return '# Launch plan\n\nThe launch review is Thursday. Original project content.';
}
