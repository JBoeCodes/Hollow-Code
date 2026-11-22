import { expect, it, vi } from 'vitest';
// @ts-expect-error: browser build has no declarations; node effects are inert.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));
import { createRoot, createSignal } from 'solid-js';
import { canvasCompareMode } from './canvasCompareMode';

it('reopens Compare on the same detail for a fresh request, retaining a local close on refresh', async () => {
  await createRoot(async dispose => {
    const [request, setRequest] = createSignal(1);
    const [comparing, setComparing] = canvasCompareMode(request, () => {});
    await Promise.resolve();
    expect(comparing()).toBe(true);
    setComparing(false);
    setRequest(1);
    expect(comparing()).toBe(false);
    setRequest(2);
    expect(comparing()).toBe(true);
    setComparing(false); setRequest(0);
    expect(comparing()).toBe(false);
    dispose();
  });
});

it('consumes a deep-link request once even when its detail is remounted', async () => {
  const [request, setRequest] = createSignal(1);
  let disposeDetail = () => {};
  let comparing = () => false;
  let close = () => {};
  const mount = () => createRoot(dispose => {
    disposeDetail = dispose;
    const mode = canvasCompareMode(request, () => setRequest(0));
    comparing = mode[0]; close = () => mode[1](false);
  });
  mount(); await Promise.resolve();
  expect(comparing()).toBe(true);
  close(); disposeDetail();
  mount(); await Promise.resolve();
  expect(comparing()).toBe(false);
  setRequest(2);
  expect(comparing()).toBe(true);
  disposeDetail();
});
