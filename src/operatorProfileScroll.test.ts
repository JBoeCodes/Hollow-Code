import { describe, expect, it } from 'vitest';
import RUNTIME from './store/runtime.ts?raw';

// Settings jumped to its top "every so often" (11-11): a `me/` file change
// refetched `operatorProfile`, a raw read of it under Settings' page Suspense
// swapped the page for its loading fallback, and the remount reset scrollTop.
describe('operatorProfile never blanks a page while it refetches', () => {
  it('is exported through preserveDuringRefetch, not as the raw resource', () => {
    const src = RUNTIME;
    expect(src).toMatch(/export const operatorProfile = preserveDuringRefetch\(_operatorProfile\)/);
    expect(src).not.toMatch(/export const \[operatorProfile\b/);
  });
});
// The wrapper's own refetch behaviour (latest, never the suspending read) is in stableResource.test.ts.
