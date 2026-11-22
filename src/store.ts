/**
 * Compatibility facade for application state.
 *
 * Existing consumers keep importing `./store` while state domains move behind
 * focused modules. New domain work should prefer the narrow modules under
 * `src/store/`; the runtime module remains the compatibility surface for state
 * that has not yet crossed a safe dependency seam.
 */
export * from './store/runtime';
export * from './store/theme';
