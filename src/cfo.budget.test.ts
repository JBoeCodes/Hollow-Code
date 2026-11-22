import { describe, expect, it } from 'vitest';
import { formatTokens, parseTokenAmount, tokenInput } from './cfo';

describe('parseTokenAmount', () => {
  it('reads plain numbers and k / M suffixes', () => {
    expect(parseTokenAmount('250000')).toBe(250000);
    expect(parseTokenAmount('500k')).toBe(500000);
    expect(parseTokenAmount('2M')).toBe(2000000);
    expect(parseTokenAmount(' 1.5m ')).toBe(1500000);
    expect(parseTokenAmount('1,200,000')).toBe(1200000);
  });
  it('treats blank as no cap', () => {
    expect(parseTokenAmount('')).toBeNull();
    expect(parseTokenAmount('   ')).toBeNull();
  });
  it('refuses everything else', () => {
    for (const text of ['0', '0k', '-5', 'abc', '2G', '1.2.3', 'k', '2 million']) expect(parseTokenAmount(text)).toBe('invalid');
  });
});

describe('formatTokens', () => {
  it('writes short amounts', () => {
    expect(formatTokens(950)).toBe('950');
    expect(formatTokens(500000)).toBe('500k');
    expect(formatTokens(1200000)).toBe('1.2M');
    expect(formatTokens(2000000)).toBe('2M');
    expect(formatTokens(20000000)).toBe('20M');
  });
  it('round-trips what it writes', () => {
    for (const n of [500000, 1200000, 2000000, 4100000]) expect(parseTokenAmount(formatTokens(n))).toBe(n);
  });
});

describe('tokenInput (L9 review)', () => {
  it('reads back exactly, so saving one field never rounds another', () => {
    expect(tokenInput(2_000_000)).toBe('2M');
    expect(tokenInput(1_234_567)).toBe('1234567');
    expect(parseTokenAmount(tokenInput(1_234_567))).toBe(1_234_567);
    expect(tokenInput(undefined)).toBe('');
    expect(formatTokens(999_950)).toBe('1M');
  });
});
