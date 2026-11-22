import { expect, it, vi } from 'vitest';
import { sendThenBackground } from './composerActions';

it('keeps the composer on a refused or review-only send', async () => {
  const open = vi.fn();
  expect(await sendThenBackground(async () => false, () => true, open)).toBe(false);
  expect(open).not.toHaveBeenCalled();
});
it('does not navigate a different conversation after send acceptance', async () => {
  let here = true;
  const open = vi.fn();
  expect(await sendThenBackground(async () => { here = false; return true; }, () => here, open)).toBe(false);
  expect(open).not.toHaveBeenCalled();
});
it('passes the origin guard through to the async destination open', async () => {
  let here = true;
  const guard = () => here;
  expect(await sendThenBackground(async () => true, guard, async current => {
    expect(current).toBe(guard);
    here = false;
    return current();
  })).toBe(false);
});
