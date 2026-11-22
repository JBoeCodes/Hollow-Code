/**
 * The Results section as cards (Canvas C5b, docs/history/CANVAS_C5.md): what each
 * card says about one Result, from what `workspace_results` already lists.
 * Pure, no DOM, no IPC.
 *
 * A card names the result's kind (with a canvas's revision), its review
 * state as words (`REVIEW_STATE`, the same words the full review uses, so
 * colour never carries it alone), whether it changed since the person's last
 * visit, its title, the conversation that wrote it and when it was updated.
 * A canvas's `.canvas.json` record, a curated section canvas's files
 * (`_plan.canvas.*`, C6 · K1) and anything in a dot folder (`.revisions/`)
 * are store bookkeeping, never a card, as in the Overview.
 */

import type { ResultReview, WorkResult } from './workspaceTypes';
import type { VaultChange } from './types';
import { REVIEW_STATE, checkPhrase, checkState, type CheckState } from './workState';
import { isResultBookkeeping } from './workspaceRecap';
import { VAULT_CHANGE_MAX_PATHS } from './sectionSource';
import type { LibraryResult } from './generated/ipc';

/** Maximum eligible previews per listing. Gallery reserves two for its detail. */
export const THUMBNAILS = 24;

export const RESULT_KIND_ICON: Readonly<Record<string, import('./icons').IconName>> = { document: 'fileText', table: 'table', image: 'fileImage', video: 'play', page: 'appWindow', canvas: 'appWindow', review: 'compare', pdf: 'fileText', data: 'fileCode', file: 'fileText' };
export const RESULT_KIND_LABEL: Readonly<Record<string, string>> = { document: 'Document', table: 'Table', image: 'Image', video: 'Video', page: 'Web page', canvas: 'Canvas', review: 'Page review', pdf: 'PDF', data: 'Data', file: 'File' };
/** A result's kind as listed: a saved canvas (Canvas C3) also names its current revision. */
export const resultKindLabel = (r: Pick<WorkResult, 'kind' | 'canvas'>) => `${RESULT_KIND_LABEL[r.kind] ?? 'File'}${r.canvas?.revision ? ` v${r.canvas.revision}` : ''}`;

/** Reviews that still need the person: everything not accepted. */
export const needsReview = (review: ResultReview) => review !== 'accepted';

export interface ResultCard {
  path: string;
  /** Workspace-relative (`outputs/…`). */
  name: string;
  title: string;
  kind: string;
  kindLabel: string;
  icon: import('./icons').IconName;
  review: ResultReview;
  reviewLabel: string;
  reviewHint: string;
  /** What the run that wrote it recorded (P1-50: a failed one is marked on the card). */
  checks: CheckState;
  checkText: string;
  /** Modified after the person's last visit; null when either time is unknown. */
  changedSinceVisit: boolean | null;
  modified: string | null;
  size: number;
  /** A saved canvas's id and current revision. */
  canvas: { id: string; revision: number | null } | null;
  /** The run that wrote it, with its conversation's title when it is in this workspace. */
  origin: { sessionKey: string | null; title: string | null; finishedAt: string } | null;
  /** What the card draws above its title: a saved canvas's outline, or a
   *  video's poster frame (inline video PR 3). */
  thumb: 'canvas' | 'poster' | 'image' | null;
}

export interface ResultsCardsModel {
  cards: ResultCard[];
  counts: {
    all: number;
    /** Not accepted yet (draft, ready, or changed since accepted). */
    toReview: number;
    accepted: number;
    /** Changed since the person's last visit; null when unknown. */
    sinceVisit: number | null;
  };
  /** The newest modified time among the cards, for the source line. */
  newest: string | null;
}

const time = (at: string | null | undefined) => { const t = at ? Date.parse(at) : NaN; return Number.isFinite(t) ? t : null; };

/** Whether a result was modified after `since`; null when either time is
 *  unknown (never a guessed "no"). */
export function changedSince(modified: string | null | undefined, since: string | null | undefined): boolean | null {
  const m = time(modified), s = time(since);
  return m === null || s === null ? null : m > s;
}

export function resultsCardsModel(input: {
  results: readonly WorkResult[];
  lastVisited: string | null;
  sessions?: readonly { key: string; title?: string | null }[];
}): ResultsCardsModel {
  const titles = new Map((input.sessions ?? []).map(s => [s.key, s.title || null]));
  const cards: ResultCard[] = input.results.filter(r => !isResultBookkeeping(r.name)).map(r => ({
    path: r.path,
    name: r.name,
    title: r.title,
    kind: r.kind,
    kindLabel: resultKindLabel(r),
    icon: RESULT_KIND_ICON[r.kind] ?? 'fileText',
    review: r.review,
    reviewLabel: REVIEW_STATE[r.review]?.label ?? r.review,
    reviewHint: REVIEW_STATE[r.review]?.hint ?? '',
    checks: checkState(r.checks),
    checkText: checkPhrase(r.checks),
    changedSinceVisit: changedSince(r.modified, input.lastVisited),
    modified: r.modified,
    size: r.size,
    canvas: r.kind === 'canvas' && r.canvas ? { id: r.canvas.id, revision: r.canvas.revision ?? null } : null,
    origin: r.origin ? { sessionKey: r.origin.sessionKey, title: r.origin.sessionKey ? titles.get(r.origin.sessionKey) ?? null : null, finishedAt: r.origin.finishedAt } : null,
    thumb: r.kind === 'canvas' && r.canvas ? 'canvas' : r.kind === 'video' ? 'poster'
      : r.kind === 'image' && /\.(png|jpe?g|gif|webp)$/i.test(r.name) && Number.isFinite(r.size) && r.size >= 0 && r.size <= 2 * 1024 * 1024 ? 'image' : null,
  }));
  const changes = cards.map(c => c.changedSinceVisit);
  let newest: string | null = null, newestAt = -Infinity;
  for (const c of cards) { const t = time(c.modified); if (t !== null && t > newestAt) { newest = c.modified; newestAt = t; } }
  return {
    cards,
    counts: {
      all: cards.length,
      toReview: cards.filter(c => needsReview(c.review)).length,
      accepted: cards.filter(c => c.review === 'accepted').length,
      sinceVisit: changes.some(c => c === null) ? null : changes.filter(Boolean).length,
    },
    newest,
  };
}

/** Cross-workspace cards use the same bookkeeping and safe-preview rules. */
export function libraryCards(list: readonly LibraryResult[]) {
  return list.flatMap(result => resultsCardsModel({ results: [result], lastVisited: null }).cards.map(card => ({
    key: JSON.stringify([result.workspaceId, result.path]),
    workspaceId: result.workspaceId, workspaceTitle: result.workspaceTitle, card,
  })));
}

/** Whether a `vault-changed` payload may have touched the workspace's
 *  `outputs/` folder: a listed path under it, or a list cut at the watcher's
 *  cap with the top folder changed. */
export function vaultChangeUnder(change: VaultChange | null | undefined, workspacePath: string): boolean {
  if (!change || !workspacePath) return false;
  const prefix = `${workspacePath}/outputs/`;
  const paths = change.paths ?? [];
  if (paths.some(p => p.startsWith(prefix))) return true;
  return paths.length >= VAULT_CHANGE_MAX_PATHS && (change.folders ?? []).includes(workspacePath.split('/')[0]);
}

/** Whether a `vault-changed` payload may have touched any workspace's
 *  `outputs/` (Library → Results, P1-19), with the same cap rule. */
export function vaultChangeUnderAnyOutputs(change: VaultChange | null | undefined): boolean {
  if (!change) return false;
  const paths = change.paths ?? [];
  if (paths.some(p => /^workspaces\/[^/]+\/outputs\//.test(p))) return true;
  return paths.length >= VAULT_CHANGE_MAX_PATHS && (change.folders ?? []).includes('workspaces');
}
