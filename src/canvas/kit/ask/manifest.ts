/** The ask kit's manifest (Canvas C4 A1, plan §5). The input components ship
 *  as their own bundle, `generated/kit-ask.min.{js,css}`, which the host
 *  inlines only into ask-canvases (`<hc-canvas ask>`), after the display kit.
 *  It is part of kit@1: the elements below are additive, and the display
 *  kit's API (`../manifest.ts`) is unchanged. A later PR adds an input by
 *  writing `components/<name>.ts`, registering it in `index.ts` and
 *  appending its tag here (the kit test checks the two agree). */

/** Every element the ask kit defines, in registration order. `hc-ask` is the
 *  form; the rest are its fields. */
export const KIT_ASK_COMPONENTS = [
  'hc-ask',
  'hc-choice',
  'hc-multi',
  'hc-range',
  'hc-rank',
  'hc-text',
  'hc-date',
  'hc-matrix',
  'hc-pick',
] as const;

export type KitAskComponent = typeof KIT_ASK_COMPONENTS[number];

/** The fields `hc-ask` collects (every ask element but the form itself). */
export const KIT_ASK_FIELDS = KIT_ASK_COMPONENTS.filter(c => c !== 'hc-ask');
