/** Hollow Canvas Kit manifest (Canvas C2, plan §3). Pure data shared by the
 *  kit bundle (`index.ts`), the host (`htmlSandbox.ts`) and the tests, so the
 *  version string and the component list live in one place.
 *
 *  The kit is host-owned: one stylesheet (`kit.css`) plus vanilla custom
 *  elements, minified by `scripts/build-canvas-kit.mjs` into `generated/` and
 *  inlined into every artifact document. A later PR adds a component by
 *  writing `components/<name>.ts`, registering it in `index.ts` and appending
 *  its tag here (the kit test checks the two agree). */

/** Bumped only for a breaking change to the markup agents write. */
export const KIT_VERSION = 'kit@1' as const;

/** Every element kit@1 defines, in registration order. */
export const KIT_COMPONENTS = [
  'hc-badge',
  'hc-callout',
  'hc-details',
  'hc-progress',
  'hc-stat',
  'hc-stats',
  'hc-steps',
  'hc-bars',
  'hc-chart',
  'hc-table',
  'hc-compare',
  'hc-tabs',
  'hc-file',
  'hc-code',
  'hc-flow',
  'hc-gauge',
  'hc-sparkline',
  'hc-timeline',
  'hc-heatmap',
  'hc-checklist',
] as const;

export type KitComponent = typeof KIT_COMPONENTS[number];

/** Tones every toned component accepts. Each carries an icon and a word, so
 *  a tone is never conveyed by colour alone. Anything else renders neutral. */
export const KIT_TONES = {
  neutral: { icon: '•', word: 'Note' },
  info: { icon: 'i', word: 'Info' },
  success: { icon: '✓', word: 'Success' },
  warning: { icon: '!', word: 'Warning' },
  danger: { icon: '✕', word: 'Danger' },
} as const;

export type KitTone = keyof typeof KIT_TONES;
