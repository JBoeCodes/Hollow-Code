// Browser-mock first-run setup (H6) and session import (H5a): the sheet's
// `first_run_*` progress and the import commands its Find what's already
// here step uses, answered from memory and localStorage. Nothing on disk is
// touched.
//
// Knobs:
// - `?firstRun=1` (or localStorage `mockFirstRun=open`): a fresh setup, every
//   step open, when no progress is stored yet. Without it the mock config has
//   no `setup` block, so setup is complete and the sheet stays shut.
// - Progress is kept in localStorage `mockFirstRunSetup`, so a reload resumes
//   at the first open step.
// - `mockSessionImport=none` (localStorage): no Claude Code or Codex folder.
import type { ImportBatch, ImportCommitResult, ImportEntry, ImportJob, ImportManifest, ImportSource, ImportSourceId } from './api';
import type { SetupStatus, SetupStepId, SetupStepState } from './firstRunApi';

const STEPS: readonly SetupStepId[] = ['context', 'find', 'sources', 'agents', 'start'];
const STORE = 'mockFirstRunSetup';
const HOME = '/Users/mock';

const COMMANDS = new Set([
  'first_run_status', 'first_run_set_step', 'first_run_reopen',
  'session_import_sources', 'inspect_session_import', 'session_import_status', 'list_session_import', 'preview_session_import',
  'commit_session_import', 'cancel_session_import',
]);

export const isMockFirstRunCommand = (command: string) => COMMANDS.has(command);

type Block = { steps: Partial<Record<SetupStepId, 'done' | 'skipped'>> };

function wantsFresh(): boolean {
  try {
    return new URLSearchParams(location.search).get('firstRun') === '1' || localStorage.getItem('mockFirstRun') === 'open';
  } catch { return false; }
}

function load(): Block | null {
  const raw = localStorage.getItem(STORE);
  if (raw) { try { return JSON.parse(raw) as Block; } catch { /* a damaged knob starts over */ } }
  if (!wantsFresh()) return null;
  const fresh: Block = { steps: {} };
  save(fresh);
  return fresh;
}

const save = (block: Block) => localStorage.setItem(STORE, JSON.stringify(block));

/** `first_run::status_of` in TypeScript: no block is complete. */
export function mockSetupStatus(block: Block | null): SetupStatus {
  const steps = STEPS.map(id => ({ id, state: (block ? block.steps[id] ?? 'open' : 'done') as SetupStepState }));
  const current = steps.find(s => s.state === 'open')?.id ?? null;
  return { version: 1, steps, current, complete: current === null };
}

// ── session import ────────────────────────────────────────────────────

const polls = new Map<string, number>();
const committed = new Set<string>();
/** Committed batches, as `session_import_status` lists them (Start reads them after a resume). */
const batches: ImportBatch[] = [];

function entry(id: string, title: string, status: ImportEntry['status'], day: number, folder: string | null): ImportEntry {
  const at = `2026-11-${String(day).padStart(2, '0')}T15:00:00Z`;
  return {
    id, externalId: `claude-code:${id}`, title, createdAt: at, lastAt: at, messageCount: 6 + day, turnCount: 3, size: 4096 + day,
    projectFolder: folder, destination: 'quick-chats', status: committed.has(id) ? 'alreadyImported' : status,
    selected: status === 'new' && !committed.has(id), sidechainLines: 0, malformedLines: 0, truncated: false, shortenedTurns: 0, contentHash: `h-${id}`,
  };
}

function manifest(importId: string): ImportManifest {
  const source: ImportSourceId = importId.includes('codex') ? 'codex' : 'claude-code';
  const label = source === 'codex' ? 'Codex' : 'Claude Code';
  const conversations = source === 'codex'
    ? [entry('cx-1', 'Fix the flaky upload test', 'new', 12, `${HOME}/Projects/uploader`)]
    : [
      entry('cc-1', 'Plan the pricing study', 'new', 14, `${HOME}/Projects/pricing-study`),
      entry('cc-2', 'Draft the launch notes', 'new', 11, null),
      entry('cc-3', 'Rename the billing module', 'alreadyImported', 8, `${HOME}/Projects/billing`),
      entry('cc-4', 'A Hollow chat', 'hollowOwn', 7, null),
    ];
  const counts = { new: 0, alreadyImported: 0, updated: 0, newVersion: 0, hollowOwn: 0, subagent: 0, unsupported: 0 };
  for (const c of conversations) counts[c.status] += 1;
  return {
    importId, source, sourceLabel: label, root: source === 'codex' ? `${HOME}/.codex/sessions` : `${HOME}/.claude/projects`,
    createdAt: '2026-11-19T09:00:00Z', state: 'ready', conversations, counts, foundFiles: conversations.length, capped: localStorage.getItem('mockSessionImportCapped') === '1',
    symlinksSkipped: 0, unreadable: 0,
  };
}

function sources(): ImportSource[] {
  const none = localStorage.getItem('mockSessionImport') === 'none';
  return [
    { source: 'claude-code', label: 'Claude Code', defaultRoot: `${HOME}/.claude/projects`, export: false, exists: !none },
    { source: 'codex', label: 'Codex', defaultRoot: `${HOME}/.codex/sessions`, export: false, exists: !none },
    { source: 'pi', label: 'Pi', defaultRoot: `${HOME}/.pi/agent/sessions`, export: false, exists: !none },
    // H5b: an export file the person picks; listed whatever is on disk.
    { source: 'chatgpt', label: 'ChatGPT', defaultRoot: null, export: true, exists: false },
  ];
}

function job(importId: string): ImportJob {
  const n = (polls.get(importId) ?? 0) + 1;
  polls.set(importId, n);
  const source: ImportSourceId = importId.includes('codex') ? 'codex' : 'claude-code';
  return n < 2
    ? { importId, source, state: 'inspecting', done: 2, total: 4 }
    : { importId, source, state: 'ready', done: 4, total: 4 };
}

export function mockFirstRunCommand(command: string, args: Record<string, unknown>): unknown {
  switch (command) {
    case 'first_run_status': return mockSetupStatus(load());
    case 'first_run_set_step': {
      const block = load();
      if (!block) throw new Error('Setup is already finished. Use Run setup again in Settings, Sources.');
      const step = String(args.step) as SetupStepId;
      const outcome = String(args.outcome);
      if (!STEPS.includes(step)) throw new Error(`Unknown setup step \`${step}\``);
      if (outcome !== 'done' && outcome !== 'skipped') throw new Error(`A setup step is done or skipped, not \`${outcome}\``);
      block.steps[step] = outcome;
      save(block);
      return mockSetupStatus(block);
    }
    case 'first_run_reopen': {
      const block: Block = { steps: {} };
      save(block);
      return mockSetupStatus(block);
    }
    case 'session_import_sources': return sources();
    case 'inspect_session_import': {
      const importId = `imp-mock-${String(args.source)}-${polls.size + 1}`;
      polls.set(importId, 0);
      return { importId, root: sources().find(s => s.source === args.source)?.defaultRoot ?? String(args.root ?? '') };
    }
    case 'session_import_status': {
      const id = args.importId ? String(args.importId) : null;
      return { jobs: id ? [job(id)] : [], batches: [...batches] };
    }
    case 'list_session_import': return manifest(String(args.importId));
    case 'preview_session_import': return { id: String(args.entry), title: 'Preview', turns: [], totalTurns: 0, sidechainLines: 0 };
    case 'commit_session_import': {
      const importId = String(args.importId);
      const ids = ((args.request as { ids?: string[] | null } | null)?.ids) ?? [];
      const list = manifest(importId).conversations;
      let imported = 0; let alreadyImported = 0;
      for (const id of ids) {
        const found = list.find(c => c.id === id);
        if (!found) continue;
        if (found.status === 'alreadyImported') alreadyImported += 1; else { imported += 1; committed.add(id); }
      }
      if (imported) batches.push({ importId, source: importId.includes('codex') ? 'codex' : 'claude-code', conversations: imported, importedAt: new Date().toISOString(), keepOutOfSearch: false });
      const result: ImportCommitResult = {
        importId, imported, updated: 0, newVersions: 0, alreadyImported, skipped: 0, failed: [],
        sessionKeys: ids.map(id => `imported-${id}`), stagingDeleted: true,
      };
      return result;
    }
    case 'cancel_session_import': { polls.delete(String(args.importId)); return true; }
    default: throw new Error(`mockFirstRun: unhandled ${command}`);
  }
}
