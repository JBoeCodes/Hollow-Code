/** Hollow Canvas Kit entry (kit@1). Bundled and minified by
 *  `scripts/build-canvas-kit.mjs` into `generated/kit.min.js`, which every
 *  artifact document inlines after the host runtime. Registration order
 *  follows `KIT_COMPONENTS`. */
import { KIT_COMPONENTS, KIT_VERSION, type KitComponent } from './manifest';
import { define, KitElement, syncScheme } from './registry';
import { HcBadge } from './components/badge';
import { HcBars } from './components/bars';
import { HcChart } from './components/chart';
import { HcCode } from './components/code';
import { HcCompare } from './components/compare';
import { HcCallout } from './components/callout';
import { HcDetails } from './components/details';
import { HcProgress } from './components/progress';
import { HcStat, HcStats } from './components/stats';
import { HcSteps } from './components/steps';
import { HcTable } from './components/table';
import { HcTabs } from './components/tabs';
import { HcFile } from './components/file';
import { HcGauge, HcSparkline, HcTimeline, HcHeatmap } from './components/instruments';
import { HcChecklist } from './components/checklist';
import { HcFlow } from './components/flow';
import { material } from './material';

const ELEMENTS: Record<KitComponent, CustomElementConstructor> = {
  'hc-badge': HcBadge,
  'hc-callout': HcCallout,
  'hc-details': HcDetails,
  'hc-progress': HcProgress,
  'hc-stat': HcStat,
  'hc-stats': HcStats,
  'hc-steps': HcSteps,
  'hc-bars': HcBars,
  'hc-chart': HcChart,
  'hc-table': HcTable,
  'hc-compare': HcCompare,
  'hc-tabs': HcTabs,
  'hc-file': HcFile,
  'hc-code': HcCode,
  'hc-flow': HcFlow,
  'hc-gauge': HcGauge,
  'hc-sparkline': HcSparkline,
  'hc-timeline': HcTimeline,
  'hc-heatmap': HcHeatmap,
  'hc-checklist': HcChecklist,
};

// The chart palette's light or dark steps follow the host theme: set now from
// the baked tokens, and again whenever the runtime applies a theme message
// (this listener is registered before any chart's, so charts redraw after).
syncScheme();
addEventListener('hollow-theme', syncScheme);

for (const name of KIT_COMPONENTS) define(name, ELEMENTS[name]);
material();

// The host's init carries the canvas's saved state: rendered components
// restore the view the person left (tab, sort, open details).
addEventListener('hollow-init', () => {
  for (const el of Array.from(document.querySelectorAll('[data-hc-ready]'))) if (el instanceof KitElement) el._restore();
});

try {
  Object.defineProperty(window, 'HollowKit', {
    value: Object.freeze({ version: KIT_VERSION, components: Object.freeze(KIT_COMPONENTS.slice()) }),
  });
} catch {
  /* already defined by an earlier copy of the kit */
}
