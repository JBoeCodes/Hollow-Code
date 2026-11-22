import type { IconName } from './icons';

/** Shared meanings, independent of the DOM factories in icons.tsx. */
export const ICON_MEANING = {
  mainAgent: 'bot',
  whatsNew: 'gift',
  memory: 'book',
  welcome: 'flag',
  ask: 'wand',
  import: 'upload',
  agent: 'bot',
  skill: 'sparkles',
  draft: 'sparkles',
} as const satisfies Record<string, IconName>;
