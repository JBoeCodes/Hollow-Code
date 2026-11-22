// The review card's words (CX20): what each found item is, where each layer
// would put it, why Accept is off and what an Accept did. PX6 adds Home and
// Stale for Claude Code's auto-memory entries, with the sort's reason. Pure, so it is
// unit-tested (`contextReview.test.ts`); the card is
// `components/ContextReviewCard.tsx`, its requests `contextReviewApi.ts`.
import type { ContextReviewAccepted, ContextReviewChoice, ContextReviewItem, ContextReviewLayer } from './generated/ipc';

export const LAYERS: readonly ContextReviewLayer[] = ['home', 'personal', 'project', 'org', 'stale'];

export function layerLabel(layer: ContextReviewLayer): string {
  switch (layer) {
    case 'home': return 'Home (this Mac)';
    case 'personal': return 'Personal';
    case 'project': return 'Project';
    case 'stale': return 'Stale (Skip it)';
    default: return 'Organization';
  }
}

/** Why an auto-memory entry got its proposed layer (PX6); `null` for any other item. */
export function sortText(item: ContextReviewItem): string | null {
  return item.sort ? `Why this layer: ${item.sort.reason}` : null;
}

const AGENTS: Record<string, string> = {
  claudeCode: 'Claude Code', codex: 'Codex', cursor: 'Cursor', gemini: 'Gemini', windsurf: 'Windsurf',
  copilot: 'Copilot', cline: 'Cline', generic: 'Shared agents',
};

const KINDS: Record<string, string> = {
  instructions: 'instructions', rule: 'rule', command: 'saved command', skill: 'skill', memory: 'memory', orgFact: 'org fact',
};

/** "Claude Code instructions", "Cursor rule", "Org fact from a chat". */
export function itemTitle(item: ContextReviewItem): string {
  if (item.source === 'org') return 'Org fact from a chat';
  const agent = item.agent ? AGENTS[item.agent] ?? item.agent : 'Agent';
  return `${agent} ${KINDS[item.kind] ?? item.kind}`;
}

/** Where it was found: "Your Claude Code folder" or "Project proj". */
export function itemOrigin(item: ContextReviewItem): string {
  if (item.source === 'org') return 'Learned in a chat; true for everyone in your organization.';
  return item.scope === 'project' ? `In the project ${item.rootLabel}` : `In your ${item.rootLabel} folder`;
}

/** Why a scanned file shows no excerpt. */
export function withheldText(code: string | null | undefined): string | null {
  switch (code) {
    case null: case undefined: case '': return null;
    case 'secret': return 'No excerpt: it looks like it holds a secret.';
    case 'finding': return 'No excerpt: a scan rule matched near its start.';
    case 'too-large': return 'No excerpt: the file is over 256 KB, so it was not read.';
    case 'read-budget': return 'No excerpt: the scan reached its read budget before this file.';
    case 'unreadable': return 'No excerpt: the file could not be read.';
    case 'empty': return 'The file is empty.';
    default: return 'No excerpt.';
  }
}

/** Why nothing can take this item; `null` when some layer may. */
export function blockedText(item: ContextReviewItem): string | null {
  switch (item.blocked) {
    case null: case undefined: return null;
    case 'secret': return 'It looks like it holds a secret, so Hollow won\'t copy it anywhere. Take the secret out, then scan again.';
    case 'block-flag': return `A blocking rule matched${item.blockedRule ? ` (${item.blockedRule})` : ''}, so Hollow won't copy it. Fix the file, then scan again.`;
    default: return 'Hollow didn\'t read this file in the scan (too large, over the read budget or unreadable), so it can\'t check what it would copy.';
  }
}

/** One scan flag, never the matched text: "Warning: rule at line 3". */
export function flagText(flag: { rule: string; severity: string; line: number }): string {
  return `${flag.severity === 'block' ? 'Blocking' : 'Warning'}: ${flag.rule}${flag.line ? ` at line ${flag.line}` : ''}`;
}

/** "Same text as CLAUDE.md in your Codex folder." for a duplicate. */
export function duplicateText(item: ContextReviewItem, all: readonly ContextReviewItem[]): string | null {
  if (!item.duplicateOf) return null;
  const first = all.find(i => i.id === item.duplicateOf);
  return first ? `Same text as ${first.relPath} (${first.rootLabel}). Accepting both keeps two copies.` : 'Same text as another file found in this scan.';
}

/** Why Accept is off for this layer; `null` when it is on. */
export function disabledText(choice: ContextReviewChoice): string | null {
  switch (choice.disabled) {
    case null: case undefined: return null;
    case 'no-personal-root': return 'Connect your personal context in Settings, Sources, Context first.';
    case 'no-org-root': return 'Connect the organization context in Settings, Sources, Context first.';
    case 'target-exists': return `${choice.target ?? 'That file'} is already in your personal context. Hollow never overwrites it.`;
    case 'not-a-project-file': return 'This file is in your own agent folder, not a project\'s repo.';
    case 'org-only': return 'An org fact goes to the organization layer, or Skip it.';
    case 'no-machine': return 'Set this Mac\'s machine name in Settings, Sources, Context first.';
    case 'stale': return `Hollow proposes Skip for this entry.${choice.detail ? ` ${choice.detail}` : ''} Skip it, or pick another layer.`;
    default: return choice.detail ?? 'That layer can\'t take this item.';
  }
}

/** What Accept into this layer does, in one sentence. */
export function choiceText(item: ContextReviewItem, choice: ContextReviewChoice): string {
  switch (choice.layer) {
    case 'personal': return `Copies it into your personal context as ${choice.target}. Nothing else is written.`;
    case 'home': return choice.target
      ? `Copies it into this Mac's home folder as ${choice.target}. Nothing else is written.`
      : 'Copies it into this Mac\'s home folder in your personal context. Nothing else is written.';
    case 'stale': return 'Skip it: nothing is written.';
    case 'project': if (item.sort) return `Needs the project lead: a suggested pull request for the project ${choice.target ?? 'it came from'}. Hollow writes nothing and opens nothing; Accept only clears it from this card.`;
      return `A suggested pull request for ${item.rootLabel}'s own repo${choice.target ? ` (${choice.target})` : ''}. Hollow writes nothing and opens nothing; Accept only clears it from this card.`;
    default: return `Opens a pull request in the organization repo${choice.target ? ` for ${choice.target}` : ''}. Hollow asks first.`;
  }
}

export function choiceFor(item: ContextReviewItem, layer: ContextReviewLayer): ContextReviewChoice | undefined {
  return item.choices.find(c => c.layer === layer);
}

/** Why Accept into `layer` is off for this item, or `null`. */
export function acceptBlocked(item: ContextReviewItem, layer: ContextReviewLayer): string | null {
  const blocked = blockedText(item);
  if (blocked) return blocked;
  const choice = choiceFor(item, layer);
  if (!choice) return 'That layer can\'t take this item.';
  return disabledText(choice);
}

/** The layers the Change layer menu offers (each with its own reason when off). */
export function layerOptions(item: ContextReviewItem): { value: ContextReviewLayer; label: string }[] {
  return LAYERS.filter(l => choiceFor(item, l)).map(l => {
    const off = disabledText(choiceFor(item, l)!);
    return { value: l, label: off ? `${layerLabel(l)} (not available)` : layerLabel(l) };
  });
}

/** The confirm an Org Accept shows first: it opens a pull request, which
 *  Hollow merges at once when the person owns the org repo (the Accept is
 *  attended, so CX8 merges an owner's PR once its checks pass). */
export function orgConfirm(item: ContextReviewItem, choice: ContextReviewChoice): { title: string; body: string; confirmLabel: string } {
  const what = item.source === 'org' ? `the org fact "${item.rootLabel}"` : `${item.relPath} from ${item.rootLabel}`;
  return {
    title: 'Open a pull request?',
    body: `Hollow will open a pull request in the organization repo that adds ${what}${choice.target ? ` as ${choice.target}` : ''}. It is scanned first. If you own the organization repo, Hollow opens the pull request and merges it at once (as soon as its checks pass); otherwise it waits for the owner's approval. Your teammates see it once it merges.`,
    confirmLabel: 'Open pull request',
  };
}

/** What one Accept did, for the card's status line. */
export function acceptedText(done: ContextReviewAccepted): string {
  switch (done.layer) {
    case 'personal': return `Copied into your personal context as ${done.target}.`;
    case 'home': return `Copied into this Mac's home folder as ${done.target}.`;
    case 'project': return 'Cleared from the card. Nothing was written: change it in the project\'s own repo.';
    default:
      if (!done.prUrl) return 'Proposed to the organization.';
      return done.prStatus === 'merged'
        ? `Pull request merged: ${done.prUrl}`
        : `Pull request opened: ${done.prUrl}${done.prReason ? ` (${done.prReason})` : ''}`;
  }
}

/** The card's open and skipped items. */
export function splitItems(items: readonly ContextReviewItem[]): { open: ContextReviewItem[]; skipped: ContextReviewItem[] } {
  return { open: items.filter(i => !i.skipped), skipped: items.filter(i => i.skipped) };
}

/** The scan line under the button. */
export function scanText(state: 'idle' | 'scanning' | 'ready' | 'cancelled' | 'failed', found: number, error?: string | null): string {
  switch (state) {
    case 'scanning': return 'Scanning your agent folders. Nothing is written.';
    case 'ready': return found === 1 ? 'Found 1 file.' : `Found ${found} files.`;
    case 'cancelled': return 'The scan was cancelled.';
    case 'failed': return `The scan stopped: ${error ?? 'unknown error'}`;
    default: return 'Looks in your Claude Code, Codex, Cursor, Gemini and Windsurf folders for instructions and memory. It reads only; each file waits for your Accept.';
  }
}
