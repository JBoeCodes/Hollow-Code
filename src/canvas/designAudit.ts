/** Host-only design check for the parts of a reply canvas the agent wrote by
 *  hand (repo ideas 2026-10-27 #1). Never imported by the kit or the prompt,
 *  and advisory only: nothing here blocks, hides or changes a canvas.
 *
 *  Two passes feed one list:
 *  - `staticDesignFindings(source)`: a pure read of the fence's own source:
 *    literal text colours, text under 11px, `transition: all`, hand-written
 *    `@keyframes` or `requestAnimationFrame`, emoji used as icons, and inline
 *    SVG without a label. The kit's own elements and stylesheet are not in
 *    the source, so they are never judged.
 *  - `measuredFindings(report)`: the frame's computed-style report (bridge
 *    `design`, untrusted and bounded by `canvasBridge.ts`): contrast against
 *    the element's real ground, rendered size, and text over a gradient,
 *    image or texture, which is never guessed at ("unresolved").
 *  `designFindings` merges and orders them (fail, unresolved, advisory) and
 *  the row shows at most `DESIGN_SHOWN`, plus a count of the rest. */

import type { DesignItem } from '../canvasBridge';
import type { PixelVerdict } from './pixelContrast';

export type DesignTier = 'fail' | 'advisory' | 'unresolved';
export type DesignRule = 'literal-color' | 'small-text' | 'transition-all' | 'motion' | 'emoji-icon' | 'svg-label' | 'contrast' | 'texture-text';
export interface DesignFinding { rule: DesignRule; tier: DesignTier; text: string; fix: string }

/** The most findings the row lists; the rest are counted. */
export const DESIGN_SHOWN = 5;
/** Text smaller than this (CSS px) fails. */
export const MIN_TEXT_PX = 11;

const TIER_ORDER: Record<DesignTier, number> = { fail: 0, unresolved: 1, advisory: 2 };
const FIX = {
  color: 'Use var(--app-ink), var(--app-body) or var(--app-muted) (or a kit tone) so the text follows light and dark themes.',
  small: 'Use at least 11px; the kit sets its own caption sizes.',
  transition: 'Name the properties that change (transition: opacity 120ms), or leave motion to the kit.',
  motion: 'Leave motion to the kit, which honours Reduce motion; a canvas draws once.',
  emoji: 'Use a kit tone (its icon and word) or a labelled inline SVG; emoji look different on every system and read aloud oddly.',
  svg: 'Add role="img" and aria-label (or a <title>), or aria-hidden="true" when it is decoration.',
  contrast: 'Use var(--app-ink) or var(--app-body) on the canvas ground, or put the text in a kit element.',
  texture: 'Keep text on a plain surface (var(--app-surface)); texture and pictures belong behind graphics, never text.',
} as const;

const KEYWORDS = new Set(['inherit', 'initial', 'unset', 'revert', 'revert-layer', 'currentcolor', 'transparent']);
const EASING = /^(ease|ease-in|ease-out|ease-in-out|linear|step-start|step-end|normal|allow-discrete|infinite)$/;
const TIME = /^-?(\d+\.?\d*|\.\d+)m?s$/;
const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const clip = (s: string, max: number) => (s.length > max ? `${s.slice(0, max - 1)}…` : s);

/** The source without comments, split into its CSS, its scripts and the
 *  rest (markup and text). */
function parts(source: string): { css: string[]; scripts: string[]; markup: string } {
  const css: string[] = [], scripts: string[] = [];
  let markup = source.replace(/<!--[\s\S]*?-->/g, ' ');
  markup = markup.replace(/<script\b[^>]*>([\s\S]*?)<\/script\s*>/gi, (_, body: string) => { scripts.push(body); return ' '; });
  markup = markup.replace(/<style\b[^>]*>([\s\S]*?)<\/style\s*>/gi, (_, body: string) => { css.push(body.replace(/\/\*[\s\S]*?\*\//g, ' ')); return ' '; });
  for (const m of markup.matchAll(/<[a-z][^>]*?\sstyle\s*=\s*("([^"]*)"|'([^']*)')/gi)) css.push(m[2] ?? m[3] ?? '');
  return { css, scripts, markup };
}

/** Each `name: value` declaration in `css` whose name is one of `names`
 *  (never the tail of a longer name such as background-color). */
function declarations(css: string, names: string): Array<{ name: string; value: string }> {
  const out: Array<{ name: string; value: string }> = [];
  const re = new RegExp(`(?:^|[;{\\s])(${names})\\s*:\\s*([^;{}]*)`, 'gi');
  for (const m of css.matchAll(re)) out.push({ name: m[1].toLowerCase(), value: m[2].replace(/!important/i, '').trim() });
  return out;
}

/** A text colour that names no token: not a var(), not a keyword. */
function literalColor(value: string): boolean {
  const v = value.trim().toLowerCase();
  return !!v && !/var\(/.test(v) && !KEYWORDS.has(v);
}

/** A px or pt size in CSS px, or null (em, rem, % and keywords are left to
 *  the frame's measured pass). */
function pxSize(value: string): number | null {
  const m = /(^|[\s/])(\d*\.?\d+)(px|pt)(?=$|[\s/,])/i.exec(value.trim());
  if (!m) return null;
  const n = Number(m[2]);
  return m[3].toLowerCase() === 'pt' ? n * 4 / 3 : n;
}

/** A `transition` value that animates every property: `all` named, or no
 *  property at all (only times and easing), which means all. */
function transitionsAll(name: string, value: string): boolean {
  if (name === 'transition-property') return value.split(',').some(p => p.trim().toLowerCase() === 'all');
  for (const segment of value.split(/,(?![^(]*\))/)) {
    const tokens = segment.trim().toLowerCase().split(/\s+(?![^(]*\))/).filter(Boolean);
    if (!tokens.length || tokens.some(t => KEYWORDS.has(t) || t === 'none')) continue;
    const named = tokens.filter(t => /^[a-z-]+$/.test(t) && !EASING.test(t));
    if (named.includes('all') || (!named.length && tokens.some(t => TIME.test(t)))) return true;
  }
  return false;
}

/** Emoji drawn as emoji: emoji-presentation characters, or a pictograph
 *  asked for as emoji (U+FE0F). Text marks such as ✓ → © are left alone. */
const EMOJI = /\p{Emoji_Presentation}|\p{Extended_Pictographic}️/gu;

/** Findings from the canvas fence's own source. Pure and deterministic. */
export function staticDesignFindings(source: string): DesignFinding[] {
  const { css, scripts, markup } = parts(source);
  const all = css.join('\n;');
  const findings: DesignFinding[] = [];
  const colours = declarations(all, 'color|-webkit-text-fill-color').filter(d => literalColor(d.value)).map(d => d.value);
  if (colours.length) {
    const examples = [...new Set(colours)].slice(0, 3).map(c => clip(c, 24)).join(', ');
    findings.push({ rule: 'literal-color', tier: 'fail', text: `${plural(colours.length, 'text colour is', 'text colours are')} written as literals (${examples}) instead of --app-* tokens.`, fix: FIX.color });
  }
  const sizes = [...declarations(all, 'font-size'), ...declarations(all, 'font')]
    .map(d => pxSize(d.value)).filter((n): n is number => n !== null && n > 0 && n < MIN_TEXT_PX);
  if (sizes.length) {
    const smallest = Math.round(Math.min(...sizes) * 10) / 10;
    findings.push({ rule: 'small-text', tier: 'fail', text: `${plural(sizes.length, 'font size is', 'font sizes are')} under ${MIN_TEXT_PX}px (smallest ${smallest}px).`, fix: FIX.small });
  }
  const transitions = declarations(all, 'transition|transition-property').filter(d => transitionsAll(d.name, d.value)).length;
  if (transitions) findings.push({ rule: 'transition-all', tier: 'advisory', text: `${plural(transitions, 'transition animates', 'transitions animate')} every property (transition: all).`, fix: FIX.transition });
  const keyframes = (all.match(/@(-webkit-)?keyframes\b/gi) ?? []).length;
  const frames = scripts.reduce((n, s) => n + (s.match(/\brequestAnimationFrame\s*\(/g) ?? []).length, 0);
  if (keyframes || frames) {
    const what = [keyframes && `${plural(keyframes, 'hand-written @keyframes rule')}`, frames && `${plural(frames, 'requestAnimationFrame call')}`].filter(Boolean).join(' and ');
    findings.push({ rule: 'motion', tier: 'advisory', text: `${what[0].toUpperCase()}${what.slice(1)}: motion of its own.`, fix: FIX.motion });
  }
  const emoji = markup.match(EMOJI) ?? [];
  if (emoji.length) findings.push({ rule: 'emoji-icon', tier: 'advisory', text: `${plural(emoji.length, 'emoji is', 'emoji are')} used as icons (${[...new Set(emoji)].slice(0, 3).join(' ')}).`, fix: FIX.emoji });
  let unlabelled = 0;
  for (const m of markup.matchAll(/<svg\b([^>]*)>([\s\S]*?)<\/svg\s*>/gi)) {
    const attrs = m[1].toLowerCase();
    if (/\saria-hidden\s*=\s*["']?true/.test(attrs) || /\srole\s*=\s*["']?(presentation|none)\b/.test(attrs)) continue;
    if (/\saria-label\s*=\s*("\s*[^"\s][^"]*"|'\s*[^'\s][^']*')/.test(attrs) || /\saria-labelledby\s*=/.test(attrs) || /<title\b/i.test(m[2])) continue;
    unlabelled++;
  }
  if (unlabelled) findings.push({ rule: 'svg-label', tier: 'advisory', text: `${plural(unlabelled, 'inline SVG has', 'inline SVGs have')} no label.`, fix: FIX.svg });
  return findings;
}

/** Findings from the frame's measured report, one per element. The report
 *  is untrusted (the canvas's own script can post one), but the bridge
 *  already bounded every field; its text is only ever shown as text. Since
 *  repo ideas 2026-10-27 #4 an unresolved text measured on the picture
 *  (`pictures`, by item index) is settled: dropped when it passes, a
 *  contrast finding when it doesn't, still unresolved (saying why) when it
 *  couldn't be scored. */
export function measuredFindings(items: readonly DesignItem[], pictures?: ReadonlyMap<number, PixelVerdict>): DesignFinding[] {
  return items.flatMap((item, i): DesignFinding[] => {
    const quote = `"${clip(item.text.replace(/\s+/g, ' ').trim(), 40)}"`;
    if (item.rule === 'contrast') {
      const ratio = Math.floor(item.ratio * 10) / 10;
      return [{ rule: 'contrast', tier: item.ratio < item.need - 0.5 ? 'fail' : 'advisory', text: `Text ${quote} has contrast ${ratio}:1 against its ground (needs ${item.need}:1).`, fix: FIX.contrast }];
    }
    if (item.rule === 'small-text') return [{ rule: 'small-text', tier: 'fail', text: `Text ${quote} renders at ${Math.round(item.size * 10) / 10}px (under ${MIN_TEXT_PX}px).`, fix: FIX.small }];
    const verdict = pictures?.get(i);
    if (verdict?.kind === 'measured') {
      if (verdict.ratio >= verdict.need) return [];
      const ratio = Math.floor(verdict.ratio * 10) / 10;
      return [{ rule: 'contrast', tier: verdict.ratio < verdict.need - 0.5 ? 'fail' : 'advisory', text: `Text ${quote} measures ${ratio}:1 on the picture, against what lies under it (needs ${verdict.need}:1).`, fix: FIX.texture }];
    }
    const why = !item.box ? ' It can\'t be measured on the picture: other text or a field shares its box, its glyphs show a clipped background, the canvas animates, or it lies outside the canvas.'
      : verdict?.kind === 'unresolved'
        ? (verdict.why === 'few-pixels' ? ' Too little of it shows to measure it on the picture.'
          : verdict.why === 'too-large' ? ' It is too large to measure on the picture.' : " It couldn't be found on the picture.")
        : '';
    return [{ rule: 'texture-text', tier: 'unresolved', text: `Text ${quote} sits on a gradient, image or texture; its contrast is unresolved.${why}`, fix: FIX.texture }];
  });
}

/** What the frame measured: its items, how many more it found past the
 *  bridge's cap, whether it looked at every text element, and (repo ideas
 *  10-27 #4) any of its unresolved texts measured on the picture. */
export interface DesignMeasure { items: readonly DesignItem[]; more: number; complete: boolean; pictures?: ReadonlyMap<number, PixelVerdict> }

/** Both passes, ordered fail → unresolved → advisory (stable within a
 *  tier), and the total: the list plus what the frame found past its cap.
 *  With a complete measured report, its rendered sizes replace the source's
 *  small-text guess: a declaration the page overrides, or CSS no text uses,
 *  isn't a problem, and the same text must not count twice. An incomplete
 *  one (a cap reached, text mid-animation, a thrown error) can't clear the
 *  source's warning, which stays, marked unverified. */
export function designFindings(source: string, report: DesignMeasure | null): { list: DesignFinding[]; total: number } {
  let found = staticDesignFindings(source);
  if (report) {
    found = report.complete
      ? found.filter(f => f.rule !== 'small-text')
      : found.map(f => (f.rule === 'small-text' ? { ...f, tier: 'unresolved' as const, text: `${f.text} Not every text was measured, so this is unverified.` } : f));
    found = [...found, ...measuredFindings(report.items, report.pictures)];
  }
  const list = found.map((f, i) => ({ f, i })).sort((a, b) => TIER_ORDER[a.f.tier] - TIER_ORDER[b.f.tier] || a.i - b.i).map(x => x.f);
  return { list, total: list.length + Math.max(0, report?.more ?? 0) };
}

/** Words for a finding's tier, shown before it. */
export const TIER_WORD: Record<DesignTier, string> = { fail: 'Fails', advisory: 'Advisory', unresolved: 'Unresolved' };

/** The design half of an Ask to fix draft: the findings shown (at most
 *  `DESIGN_SHOWN`), each with its fix, and the count of the rest (counted
 *  ones included). Plain text: angle brackets are dropped, as in
 *  `canvasLayoutRequest`. */
export function designRequestLines(findings: readonly DesignFinding[], total: number): string {
  const plain = (s: string) => s.replace(/[<>]/g, '');
  const shown = findings.slice(0, DESIGN_SHOWN);
  const rows = shown.map((f, i) => `${i + 1}. ${TIER_WORD[f.tier]}: ${plain(f.text)} Fix: ${plain(f.fix)}`);
  if (total > shown.length) rows.push(`And ${plural(total - shown.length, 'more design problem')}.`);
  return rows.join('\n');
}

/** The Ask to fix draft for a row with design findings, or with layout
 *  findings in more than one diagram (one diagram alone keeps
 *  `canvasLayoutRequest`'s words). Bounded like it (2,000 characters) and
 *  plain text; it reserves the same-id instruction. */
export function canvasFindingsRequest(
  root: { id: string; title: string },
  layout: readonly { label: string; findings: readonly { text: string }[] }[],
  design: { list: readonly DesignFinding[]; total: number },
): string {
  const plain = (s: string, max: number) => clip(s.replace(/[<>]/g, '').replace(/\s+/g, ' ').trim(), max);
  const ending = '\nPlease send a corrected hollow-canvas fence with the same id. Layout: shorter labels, groups ([Name] … []) or direction="right". Design: --app-* tokens for text colours, text of at least 11px on a plain surface, and motion left to the kit.';
  const heading = `The canvas "${plain(root.title, 120)}" (id ${plain(root.id, 64)}) has these findings:\n`;
  const budget = 2000 - heading.length - ending.length;
  const layoutText = layout.map(g => `In the diagram "${plain(g.label, 120)}":\n${g.findings.map((f, i) => `${i + 1}. ${plain(f.text, 300)}`).join('\n')}`).join('\n');
  // Counted findings past the listed ones still go in (as "And N more").
  const designText = design.total ? `In the hand-written HTML and CSS:\n${designRequestLines(design.list, design.total)}` : '';
  // Each part keeps its share: many layout findings never push the design
  // findings (the ones the row shows) out of the draft, nor the reverse.
  const joiner = layoutText && designText ? 1 : 0;
  const designShare = Math.min(designText.length, Math.max(Math.floor((budget - joiner) / 2), budget - joiner - layoutText.length));
  const layoutPart = clip(layoutText, budget - joiner - designShare);
  return heading + [layoutPart, clip(designText, budget - joiner - layoutPart.length)].filter(Boolean).join('\n') + ending;
}
