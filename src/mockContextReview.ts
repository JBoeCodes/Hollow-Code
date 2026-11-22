// Browser-mock context scan (CX19) and review card (CX20): the scan
// commands and `context_review_*`, answered from one in-memory fixture under
// /Users/mock. Nothing on disk is touched. Before a scan the card lists only
// queued org facts (none by default), so Learnings opens as it did.
//
// Knobs (localStorage):
// - `mockContextLayers=connected`: the personal and org clones are set up
//   (default: neither, so Accept for Personal and Org says why it is off).
// - `mockContextReview=org`: one queued org fact from a chat.
// After a scan two Claude Code auto-memory entries (PX6) list too: one sorted
// Home, one Stale.
import type { ContextReviewAccepted, ContextReviewChoice, ContextReviewItem, ContextReviewLayer, ContextReviewList } from './generated/ipc';

const HOME = '/Users/mock';
const SCAN_ID = 'imp-20261119-0000c20a';
const get = (k: string) => localStorage.getItem(k);

const COMMANDS = new Set([
  'context_scan_sources', 'start_context_scan', 'context_scan_status', 'list_context_scan', 'cancel_context_scan',
  'context_review_list', 'context_review_accept', 'context_review_skip', 'context_review_unskip',
]);

export const isMockContextReviewCommand = (command: string) => COMMANDS.has(command);

let scanned = false;
let polls = 0;
const skipped = new Set<string>();
const accepted = new Set<string>();
const written = new Set<string>();

function choice(layer: ContextReviewLayer, target: string | null, disabled: ContextReviewChoice['disabled'] = null): ContextReviewChoice {
  return { layer, target, disabled, detail: null, outward: layer === 'org' };
}

function scanItem(id: string, over: Partial<ContextReviewItem> & { relPath: string }): ContextReviewItem {
  const connected = get('mockContextLayers') === 'connected';
  const user = (over.scope ?? 'user') === 'user';
  const agentDir = over.agent === 'codex' ? 'codex' : 'claude-code';
  const personal = `memory/imported/${agentDir}/${user ? '' : 'pricing-study/'}${over.relPath.replace(/^\./, '')}${over.relPath.endsWith('.md') ? '' : '.md'}`;
  return {
    id, source: 'scan', agent: 'claudeCode', kind: 'instructions', scope: 'user', rootLabel: 'Claude Code',
    path: `${HOME}/.claude/${over.relPath}`, size: 180, modified: 1763500000, excerpt: null, withheld: null, secretLike: false,
    flags: [], scanNotes: [], duplicateOf: null, chat: null, proposedLayer: user ? 'personal' : 'project', sort: null, blocked: null, blockedRule: null,
    skipped: skipped.has(id),
    choices: [
      choice('personal', personal, !connected ? 'no-personal-root' : written.has(personal) ? 'target-exists' : null),
      choice('project', user ? null : `${HOME}/Hollow/Projects/pricing-study`, user ? 'not-a-project-file' : null),
      choice('org', `ai-tools/imported/${agentDir}/${over.relPath.replace(/^\./, '')}`, connected ? null : 'no-org-root'),
    ],
    ...over,
  } as ContextReviewItem;
}

/** A Claude Code auto-memory entry with its PX6 sort. */
function memoryItem(id: string, file: string, excerpt: string, sort: NonNullable<ContextReviewItem['sort']>): ContextReviewItem {
  const connected = get('mockContextLayers') === 'connected';
  const relPath = `projects/-Users-mock-Hollow-Projects-pricing-study/memory/${file}`;
  const home = `home/mock-mac/${file}`;
  const base = scanItem(id, { relPath, kind: 'memory', excerpt, proposedLayer: sort.layer, sort });
  const choices: ContextReviewChoice[] = [
    base.choices[0],
    choice('project', '-Users-mock-Hollow-Projects-pricing-study'),
    base.choices[2],
    choice('home', home, !connected ? 'no-personal-root' : written.has(home) ? 'target-exists' : null),
  ];
  if (sort.layer === 'stale') choices.push({ ...choice('stale', null, 'stale'), detail: sort.reason });
  return { ...base, choices };
}

function items(): ContextReviewItem[] {
  const out: ContextReviewItem[] = [];
  if (scanned) {
    out.push(
      scanItem('ctx-c20a00000001', { relPath: 'CLAUDE.md', excerpt: 'Use tabs. Run the tests before every commit. Keep replies short.' }),
      scanItem('ctx-c20a00000002', { relPath: 'commands/ship.md', kind: 'command', excerpt: 'Ship the branch once the gate passes and the PR is green.' }),
      scanItem('ctx-c20a00000003', { relPath: 'memory/keys.md', kind: 'memory', withheld: 'secret', secretLike: true, blocked: 'secret', flags: [{ rule: 'anthropic-key', group: 'secret', severity: 'block', line: 2 }] }),
      scanItem('ctx-c20a00000004', { relPath: 'AGENTS.md', agent: 'codex', rootLabel: 'Codex', path: `${HOME}/.codex/AGENTS.md`, excerpt: 'Use tabs. Run the tests before every commit. Keep replies short.', duplicateOf: 'ctx-c20a00000001' }),
      scanItem('ctx-c20a00000005', { relPath: 'CLAUDE.md', scope: 'project', agent: 'claudeCode', rootLabel: 'pricing-study', path: `${HOME}/Hollow/Projects/pricing-study/CLAUDE.md`, excerpt: 'This project builds with make and deploys from main.' }),
      memoryItem('ctx-c20a00000006', 'runner.md', 'The mac mini runner keeps its data on the external SSD; check free space first.', {
        layer: 'home', rule: 'disk-runner', reason: 'Home by rules disk-runner and named-machine: it names disks, cores, memory or runners and names this Mac or another machine (4 signs).',
      }),
      memoryItem('ctx-c20a00000007', 'MEMORY.md', '- [Runner](runner.md): where the runner lives', {
        layer: 'stale', rule: 'index', reason: 'Stale by rule index: MEMORY.md is the folder\'s index of the other entries, not an entry.',
      }),
    );
  }
  if (get('mockContextReview') === 'org') {
    const connected = get('mockContextLayers') === 'connected';
    out.push({
      id: 'org-c20a00000001', source: 'org', agent: null, kind: 'orgFact', scope: null, rootLabel: 'Add Refunds',
      path: 'business-rules/refunds.md', relPath: 'business-rules/refunds.md', size: 52, modified: 1763500000,
      excerpt: 'Refunds go to the original payment.', withheld: null, secretLike: false, flags: [], scanNotes: [], duplicateOf: null,
      chat: 'mock-chat-1', proposedLayer: 'org', sort: null, blocked: null, blockedRule: null, skipped: skipped.has('org-c20a00000001'),
      choices: [choice('personal', null, 'org-only'), choice('project', null, 'org-only'), choice('org', 'business-rules/refunds.md', connected ? null : 'no-org-root')],
    });
  }
  return out.filter(i => !accepted.has(i.id));
}

function list(): ContextReviewList {
  return { scanId: scanned ? SCAN_ID : null, items: items(), accepted: accepted.size };
}

export function mockContextReviewCommand(command: string, args: Record<string, unknown>): unknown {
  switch (command) {
    case 'context_scan_sources':
      return [['Claude Code', '.claude'], ['Codex', '.codex'], ['Cursor', '.cursor'], ['Gemini', '.gemini'], ['Windsurf', '.codeium/windsurf']]
        .map(([label, dir]) => ({ label, path: `${HOME}/${dir}`, scope: 'user', exists: label !== 'Windsurf' }));
    case 'start_context_scan':
      polls = 0;
      return { scanId: SCAN_ID, roots: [`${HOME}/.claude`, `${HOME}/.codex`] };
    case 'context_scan_status': {
      if (args.scanId !== SCAN_ID) return null;
      // One "scanning" answer, then ready.
      if (polls++ < 1) return { scanId: SCAN_ID, state: 'scanning', found: 0 };
      scanned = true;
      return { scanId: SCAN_ID, state: 'ready', found: 7 };
    }
    case 'list_context_scan':
      if (!scanned) throw new Error('That scan is no longer here; run it again.');
      return { scanId: SCAN_ID, state: 'ready', roots: [], proposals: [], secretFiles: 1, bytesRead: 900 };
    case 'cancel_context_scan': { const had = scanned; scanned = false; return had; }
    case 'context_review_list': return list();
    case 'context_review_accept': {
      const id = String(args.id ?? '');
      const layer = args.layer as ContextReviewLayer;
      const item = items().find(i => i.id === id);
      if (!item) throw new Error('That item is no longer here; scan again.');
      if (item.blocked) throw new Error('This file looks like it holds a secret, so Hollow won\'t copy it anywhere.');
      const c = item.choices.find(x => x.layer === layer);
      if (!c || c.disabled) throw new Error('That layer can\'t take this item.');
      accepted.add(id);
      if ((layer === 'personal' || layer === 'home') && c.target) written.add(c.target);
      const done: ContextReviewAccepted = {
        id, layer, target: c.target, written: layer === 'personal' || layer === 'home',
        prUrl: layer === 'org' ? 'https://github.com/mock-org/org-context/pull/12' : null,
        prStatus: layer === 'org' ? 'waiting' : null, prReason: layer === 'org' ? 'needs owner approval' : null,
      };
      return done;
    }
    case 'context_review_skip': { const id = String(args.id ?? ''); const had = skipped.has(id); skipped.add(id); return !had; }
    case 'context_review_unskip': return skipped.delete(String(args.id ?? ''));
    default: throw new Error(`mockContextReview: unknown command ${command}`);
  }
}
