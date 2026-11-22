import { describe, expect, it } from 'vitest';
import { perMillion, priceFromForm } from './modelPrices';

// Overnight 9 A13: Settings, Usage, Prices, the add-or-change form.
describe('priceFromForm', () => {
  const ok = { model: 'gpt-test-codex', input: '2', cacheRead: '0.2', output: '10', source: 'test price', asOf: '2026-10-19' };

  it('makes a price, a cache write at the input rate', () => {
    expect(priceFromForm({ ...ok, model: ' gpt-test-codex ' })).toEqual({
      model: 'gpt-test-codex', input: 2, cacheWrite: 2, cacheRead: 0.2, output: 10, source: 'test price', asOf: '2026-10-19',
    });
    expect(priceFromForm({ ...ok, cacheRead: '0' })).toMatchObject({ cacheRead: 0 });
    // A Claude row keeps the one-hour cache write, 2× input, as the built-ins.
    expect(priceFromForm({ ...ok, model: 'claude-opus-5-5', input: '4' })).toMatchObject({ input: 4, cacheWrite: 8 });
  });

  it('names the first thing wrong, in the backend’s terms', () => {
    expect(priceFromForm({ ...ok, model: 'GPT 6' })).toMatch(/^Enter a model id/);
    expect(priceFromForm({ ...ok, input: '' })).toBe('Input must be 0 to 1,000 dollars per million tokens.');
    expect(priceFromForm({ ...ok, cacheRead: '-1' })).toMatch(/^Cached input must be/);
    expect(priceFromForm({ ...ok, output: '1001' })).toMatch(/^Output must be/);
    expect(priceFromForm({ ...ok, output: 'ten' })).toMatch(/^Output must be/);
    expect(priceFromForm({ ...ok, source: '  ' })).toMatch(/^Say where the price comes from/);
    expect(priceFromForm({ ...ok, asOf: '' })).toBe('Enter the date the price was read.');
  });

  it('shows a per-million price without trailing zeros', () => {
    expect([perMillion(2), perMillion(0.2), perMillion(0.0125), perMillion(1000)]).toEqual(['$2', '$0.2', '$0.0125', '$1,000']);
  });
});
