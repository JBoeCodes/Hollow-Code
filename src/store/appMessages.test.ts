import { afterEach, expect, it } from 'vitest';
import { appMessages, recordAppMessage, setAppMessages } from './appMessages';
afterEach(() => setAppMessages([]));

it('retains the newest fifty messages, including failure detail, without callbacks', () => {
  for (let i = 0; i < 55; i++) recordAppMessage(`Message ${i}`, i === 5, i === 5 ? 'Original failure' : undefined);
  expect(appMessages()).toHaveLength(50);
  expect(appMessages()[0].message).toBe('Message 54');
  expect(appMessages()[49]).toMatchObject({ message: 'Message 5', failed: true, detail: 'Original failure' });
  expect(appMessages().some(m => m.message === 'Message 4')).toBe(false);
});
