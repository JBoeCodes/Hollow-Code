/**
 * A saved canvas described in words, for its card in the Results section
 * (Canvas C5b): "3 stats · 1 chart · 1 table". Derived from the fence body
 * by the fence parser (`parseCanvasFence`, then `innerElementCounts`) and
 * named by the kit manifest, so a card can say what a canvas holds without
 * rendering it: no agent markup reaches the host DOM and no frame is spent.
 * Pure, no DOM.
 */

import { innerElementCounts, parseCanvasFence } from './fence';
import { KIT_COMPONENTS } from './kit/manifest';

/** What each part is called, singular and plural, in display order. Kit
 *  containers that only group others (`hc-stats`) are not named. Plain
 *  HTML counts too: a `<table>` is a table, `<svg>`/`<canvas>` a drawing. */
const PARTS: readonly { tags: readonly string[]; one: string; many: string }[] = [
  { tags: ['hc-stat'], one: 'stat', many: 'stats' },
  { tags: ['hc-chart'], one: 'chart', many: 'charts' },
  { tags: ['hc-flow'], one: 'diagram', many: 'diagrams' },
  { tags: ['hc-bars'], one: 'bar list', many: 'bar lists' },
  { tags: ['hc-table', 'table'], one: 'table', many: 'tables' },
  { tags: ['hc-progress'], one: 'progress bar', many: 'progress bars' },
  { tags: ['hc-steps'], one: 'timeline', many: 'timelines' },
  { tags: ['hc-compare'], one: 'comparison', many: 'comparisons' },
  { tags: ['hc-callout'], one: 'callout', many: 'callouts' },
  { tags: ['hc-tabs'], one: 'tab set', many: 'tab sets' },
  { tags: ['hc-details'], one: 'fold', many: 'folds' },
  { tags: ['hc-code'], one: 'code block', many: 'code blocks' },
  { tags: ['hc-file'], one: 'file link', many: 'file links' },
  { tags: ['hc-badge'], one: 'badge', many: 'badges' },
  { tags: ['svg', 'canvas'], one: 'drawing', many: 'drawings' },
  { tags: ['img'], one: 'image', many: 'images' },
];

export interface CanvasStructure {
  /** The named parts, in display order, with their counts. */
  parts: { label: string; count: number }[];
  /** How many kit elements kit@1 defines appear (containers included). */
  kitElements: number;
  /** Elements whose tag starts `hc-` but that kit@1 doesn't define. */
  unknownKit: number;
  /** The canvas carries its own scripts. */
  scripted: boolean;
}

/** The structure of a fence body, or null when it isn't a ready canvas. */
export function canvasStructure(body: string): CanvasStructure | null {
  const p = parseCanvasFence(body, true);
  if (p.state !== 'ready') return null;
  return structureOfInner(p.inner);
}

export function structureOfInner(inner: string): CanvasStructure {
  const counts = innerElementCounts(inner);
  const parts: CanvasStructure['parts'] = [];
  for (const part of PARTS) {
    const count = part.tags.reduce((n, tag) => n + (counts.get(tag) ?? 0), 0);
    if (count) parts.push({ label: `${count} ${count === 1 ? part.one : part.many}`, count });
  }
  const known = new Set<string>(KIT_COMPONENTS);
  let kitElements = 0, unknownKit = 0;
  for (const [tag, n] of counts) {
    if (known.has(tag)) kitElements += n;
    else if (tag.startsWith('hc-')) unknownKit += n;
  }
  return { parts, kitElements, unknownKit, scripted: (counts.get('script') ?? 0) > 0 };
}

/** One line for a card: up to `max` parts, then "+N more"; a canvas with
 *  none of the named parts reads "Text and layout". */
export function structureSummary(s: CanvasStructure, max = 4): string {
  if (!s.parts.length) return 'Text and layout';
  const shown = s.parts.slice(0, max).map(p => p.label);
  const rest = s.parts.length - shown.length;
  return rest > 0 ? `${shown.join(' · ')} · +${rest} more` : shown.join(' · ');
}
