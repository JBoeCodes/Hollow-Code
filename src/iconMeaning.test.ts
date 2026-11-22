import { describe, expect, it } from 'vitest';
import { ACTION_DEFS } from './actionDefs';
import { KIND_META } from './vaultKinds';
import { ICON_MEANING } from './iconMeaning';

describe('icon meanings', () => {
  it('keeps every sparkles use in the capped meaning map', () => {
    const sources=import.meta.glob(['./**/*.{ts,tsx}','!./**/*.test.*','!./icons.tsx','!./iconMeaning.ts'],{query:'?raw',import:'default',eager:true}) as Record<string,string>;
    expect(Object.keys(sources).filter(p=>/['"]sparkles['"]/.test(sources[p]))).toEqual([]);
  });
  it('distinguishes COO, Ask and Memory (P1-56)', () => {
    const main = ACTION_DEFS.find(a => a.label === 'COO')!;
    const ask = ACTION_DEFS.find(a => a.id === 'ask-vault')!;
    expect(main).toBeDefined();
    expect(ask).toBeDefined();
    expect(new Set([main.icon, ask.icon, KIND_META.learning.icon]).size).toBe(3);
  });
  it('limits sparkles to at most three action meanings (P1-56)', () => {
    expect(ACTION_DEFS.filter(a => a.icon === 'sparkles').length).toBeLessThanOrEqual(3);
    expect(Object.values(ICON_MEANING).filter(icon => icon === 'sparkles').length).toBeLessThanOrEqual(3);
  });
});
