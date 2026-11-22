import { describe, expect, it } from 'vitest';
import type { ContextReviewChoice, ContextReviewItem } from './generated/ipc';
import {
  choiceFor, acceptBlocked, acceptedText, blockedText, choiceText, disabledText, duplicateText, flagText, itemTitle, layerLabel, layerOptions, orgConfirm, scanText, sortText, splitItems, withheldText,
} from './contextReview';
import API from './contextReviewApi.ts?raw';
import CARD from './components/ContextReviewCard.tsx?raw';

const choice = (layer: ContextReviewChoice['layer'], over: Partial<ContextReviewChoice> = {}): ContextReviewChoice =>
  ({ layer, target: null, disabled: null, detail: null, outward: layer === 'org', ...over });

function item(over: Partial<ContextReviewItem> = {}): ContextReviewItem {
  return {
    id: 'ctx-000000000001', source: 'scan', agent: 'claudeCode', kind: 'instructions', scope: 'user', rootLabel: 'Claude Code',
    path: '/Users/mock/.claude/CLAUDE.md', relPath: 'CLAUDE.md', size: 40, modified: null, excerpt: 'Use tabs.', withheld: null,
    secretLike: false, flags: [], scanNotes: [], duplicateOf: null, chat: null, proposedLayer: 'personal', sort: null,
    choices: [
      choice('personal', { target: 'memory/imported/claude-code/CLAUDE.md' }),
      choice('project', { disabled: 'not-a-project-file' }),
      choice('org', { target: 'ai-tools/imported/claude-code/CLAUDE.md' }),
    ],
    blocked: null, blockedRule: null, skipped: false, ...over,
  };
}

describe('context review card words', () => {
  it('names the item, its scan flags and why an excerpt is missing', () => {
    expect(itemTitle(item())).toBe('Claude Code instructions');
    expect(itemTitle(item({ agent: 'cursor', kind: 'rule' }))).toBe('Cursor rule');
    expect(itemTitle(item({ source: 'org', kind: 'orgFact', agent: null }))).toBe('Org fact from a chat');
    expect(flagText({ rule: 'injection-phrase', severity: 'warn', line: 3 })).toBe('Warning: injection-phrase at line 3');
    expect(flagText({ rule: 'x', severity: 'block', line: 0 })).toBe('Blocking: x');
    expect(withheldText('secret')).toMatch(/secret/);
    expect(withheldText(null)).toBeNull();
  });

  it('refuses a secret-like or blocked item on every layer, and says why', () => {
    const secret = item({ blocked: 'secret', secretLike: true });
    for (const layer of ['personal', 'project', 'org'] as const) expect(acceptBlocked(secret, layer)).toMatch(/secret/);
    const flagged = item({ blocked: 'block-flag', blockedRule: 'hidden-text (line 2)' });
    expect(blockedText(flagged)).toContain('hidden-text (line 2)');
    expect(acceptBlocked(item(), 'personal')).toBeNull();
  });

  it('says why a layer is off: no clone, an existing file, not a project file', () => {
    expect(disabledText(choice('personal', { disabled: 'no-personal-root' }))).toBe('Connect your personal context in Settings, Sources, Context first.');
    expect(disabledText(choice('org', { disabled: 'no-org-root' }))).toMatch(/organization context in Settings, Sources, Context/);
    expect(disabledText(choice('personal', { disabled: 'target-exists', target: 'memory/imported/x.md' }))).toMatch(/never overwrites/);
    expect(disabledText(choice('org', { disabled: 'refused', detail: 'Path refused.' }))).toBe('Path refused.');
    expect(acceptBlocked(item(), 'project')).toMatch(/not a project/);
    expect(layerOptions(item()).map(o => o.label)).toEqual(['Personal', 'Project (not available)', 'Organization']);
  });

  it('describes each layer: Project writes nothing, Org asks first', () => {
    const p = item({ scope: 'project', rootLabel: 'proj' });
    expect(choiceText(p, choice('project', { target: '/Users/mock/proj' }))).toMatch(/Hollow writes nothing and opens nothing/);
    expect(choiceText(item(), choice('org', { target: 'ai-tools/x.md' }))).toMatch(/pull request.*asks first/);
    const confirm = orgConfirm(item(), choice('org', { target: 'ai-tools/x.md' }));
    expect(confirm.confirmLabel).toBe('Open pull request');
    expect(confirm.body).toMatch(/If you own the organization repo, Hollow opens the pull request and merges it at once/);
    expect(confirm.body).toMatch(/otherwise it waits for the owner's approval/);
    expect(confirm.body).not.toMatch(/the repo's owner reviews it/);
  });

  it('marks a duplicate and splits skipped items', () => {
    const first = item();
    const dup = item({ id: 'ctx-000000000002', duplicateOf: first.id, rootLabel: 'Codex' });
    expect(duplicateText(dup, [first, dup])).toMatch(/Same text as CLAUDE.md \(Claude Code\)/);
    expect(duplicateText(first, [first, dup])).toBeNull();
    const { open, skipped } = splitItems([first, { ...dup, skipped: true }]);
    expect(open.map(i => i.id)).toEqual([first.id]);
    expect(skipped.map(i => i.id)).toEqual([dup.id]);
  });

  it('says what an Accept did', () => {
    const base = { id: 'x', target: null, written: false, prUrl: null, prStatus: null, prReason: null };
    expect(acceptedText({ ...base, layer: 'personal', target: 'memory/imported/a.md', written: true })).toMatch(/memory\/imported\/a\.md/);
    expect(acceptedText({ ...base, layer: 'project' })).toMatch(/Nothing was written/);
    expect(acceptedText({ ...base, layer: 'org', prUrl: 'https://x/pull/1', prStatus: 'waiting', prReason: 'needs owner approval' })).toBe('Pull request opened: https://x/pull/1 (needs owner approval)');
    expect(scanText('ready', 1)).toBe('Found 1 file.');
    expect(scanText('scanning', 0)).toMatch(/Nothing is written/);
  });

  it('shows an auto-memory entry\'s sort: Home copies, Stale proposes Skip, Project needs its lead (PX6)', () => {
    const reason = 'Stale by rule index: MEMORY.md is the folder\'s index of the other entries, not an entry.';
    const memory = (layer: ContextReviewChoice['layer'], extra: ContextReviewChoice[] = []) => item({
      kind: 'memory', relPath: 'projects/-tmp-app/memory/MEMORY.md', proposedLayer: layer,
      sort: { layer, rule: layer === 'stale' ? 'index' : 'home-path', reason },
      choices: [
        choice('personal', { target: 'memory/imported/claude-code/projects/-tmp-app/memory/MEMORY.md' }),
        choice('project', { target: '-tmp-app' }),
        choice('org', { target: 'ai-tools/imported/claude-code/projects/-tmp-app/memory/MEMORY.md' }),
        choice('home', { target: 'home/macbook/MEMORY.md' }),
        ...extra,
      ],
    });
    const stale = memory('stale', [choice('stale', { disabled: 'stale', detail: reason })]);
    expect(sortText(stale)).toBe(`Why this layer: ${reason}`);
    expect(sortText(item())).toBeNull();
    expect(acceptBlocked(stale, 'stale')).toBe(`Hollow proposes Skip for this entry. ${reason} Skip it, or pick another layer.`);
    expect(acceptBlocked(stale, 'personal')).toBeNull();
    expect(choiceText(stale, choice('stale'))).toBe('Skip it: nothing is written.');
    expect(layerOptions(stale).map(o => o.label)).toEqual(['Home (this Mac)', 'Personal', 'Project', 'Organization', 'Stale (Skip it) (not available)']);
    expect(layerLabel('home')).toBe('Home (this Mac)');

    const home = memory('home');
    expect(choiceText(home, choiceFor(home, 'home')!)).toBe('Copies it into this Mac\'s home folder as home/macbook/MEMORY.md. Nothing else is written.');
    expect(choiceText(home, choiceFor(home, 'project')!)).toMatch(/^Needs the project lead: a suggested pull request for the project -tmp-app\. Hollow writes nothing and opens nothing/);
    expect(disabledText(choice('home', { disabled: 'no-machine' }))).toMatch(/machine name in Settings, Sources, Context/);
    const base = { id: 'x', target: 'home/macbook/a.md', written: true, prUrl: null, prStatus: null, prReason: null };
    expect(acceptedText({ ...base, layer: 'home' })).toBe('Copied into this Mac\'s home folder as home/macbook/a.md.');
  });

  it('has no batch accept: one id per call, no select-all on the card', () => {
    expect(API).toMatch(/contextReviewAccept = \(id: string, layer/);
    expect(API).not.toMatch(/ids\s*:|string\[\]\)|Batch/);
    expect(CARD).not.toMatch(/Select all|selectAll|type="checkbox"/);
  });
});
