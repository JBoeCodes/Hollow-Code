/** Network-backed connector state. Imports from here make connector coupling
 * visible without changing the singleton signals owned by `plaud.ts` (re-exported by
 * `runtime.ts`). */
export {
  plaudStatus, plaudMessage, checkPlaudStatus, connectPlaud, disconnectPlaud,
} from './runtime';
export type { PlaudStatus } from './runtime';
