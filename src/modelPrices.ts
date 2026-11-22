/**
 * Settings, Usage, Prices (overnight 9 A13): the add-or-change form's pure
 * model. The rules mirror the backend's (`usage_prices::validate`), so a
 * mistake shows beside the form before the confirm step; the backend still
 * checks. The panel is `settings/PricesPanel.tsx`.
 */
import type { ModelPrice } from './types';

/** The form's fields, as typed. */
export type PriceForm = Partial<Record<'model' | 'input' | 'cacheRead' | 'output' | 'source' | 'asOf', string>>;

/** No model costs more than this per million tokens (as the backend). */
const PRICE_MAX = 1000;

/** The form as a price, or the first thing wrong with it, in words. */
export function priceFromForm(f: PriceForm): ModelPrice | string {
  const model = (f.model ?? '').trim();
  if (!/^[a-z0-9._-]{1,80}$/.test(model)) return 'Enter a model id (lowercase letters, digits, “-”, “.”, “_”).';
  const n = (key: 'input' | 'cacheRead' | 'output') => Number((f[key] ?? '').trim());
  for (const [key, label] of [['input', 'Input'], ['cacheRead', 'Cached input'], ['output', 'Output']] as const) {
    const v = n(key);
    if ((f[key] ?? '').trim() === '' || !Number.isFinite(v) || v < 0 || v > PRICE_MAX) {
      return `${label} must be 0 to 1,000 dollars per million tokens.`;
    }
  }
  const source = (f.source ?? '').trim();
  if (!source || source.length > 200) return 'Say where the price comes from (up to 200 characters).';
  const asOf = (f.asOf ?? '').trim();
  if (!/^\d{4}-\d{2}-\d{2}$/.test(asOf)) return 'Enter the date the price was read.';
  // Codex reports no cache writes, so a write is priced at input. Claude
  // Code writes the one-hour cache, 2× input, as the built-in rows: a row
  // replacing one keeps that rate (the turn guards' running estimate uses it).
  const write = model.startsWith('claude-') ? n('input') * 2 : n('input');
  return { model, input: n('input'), cacheWrite: write, cacheRead: n('cacheRead'), output: n('output'), source, asOf };
}

/** A per-million price as dollars: "$2", "$0.2", "$0.0125". */
export function perMillion(v: number): string {
  return `$${v.toLocaleString('en-US', { maximumFractionDigits: 4 })}`;
}
