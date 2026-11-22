/** Hollow Canvas Kit, ask inputs (kit@1, Canvas C4 A1). Bundled by
 *  `scripts/build-canvas-kit.mjs` into `generated/kit-ask.min.js`, which the
 *  host inlines only into ask-canvases, after the display kit. Registration
 *  order follows `KIT_ASK_COMPONENTS`. */
import { define } from '../registry';
import { KIT_ASK_COMPONENTS, KIT_ASK_FIELDS, type KitAskComponent } from './manifest';
import type { AskField } from './field';
import { HcAsk } from './components/ask';
import { HcChoice } from './components/choice';
import { HcMulti } from './components/multi';
import { HcRange } from './components/range';
import { HcRank } from './components/rank';
import { HcText } from './components/text';
import { HcDate } from './components/date';
import { HcMatrix } from './components/matrix';
import { HcPick } from './components/pick';

const ELEMENTS: Record<KitAskComponent, CustomElementConstructor> = {
  'hc-ask': HcAsk,
  'hc-choice': HcChoice,
  'hc-multi': HcMulti,
  'hc-range': HcRange,
  'hc-rank': HcRank,
  'hc-text': HcText,
  'hc-date': HcDate,
  'hc-matrix': HcMatrix,
  'hc-pick': HcPick,
};

for (const name of KIT_ASK_COMPONENTS) define(name, ELEMENTS[name]);

// The host's init carries the canvas's saved state: drawn fields show the
// answers the person chose before this load (P1-33). The display kit's own
// restore skips them (each bundle has its own element base).
addEventListener('hollow-init', () => {
  for (const el of Array.from(document.querySelectorAll<AskField>(KIT_ASK_FIELDS.join(',')))) if (el.hasAttribute('data-hc-ready')) el._restore();
});

// Display-kit wrappers can upgrade their static fallback once the ask fields exist.
dispatchEvent(new Event('hc-ask-ready'));
