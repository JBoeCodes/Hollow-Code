import { describe, expect, it } from 'vitest';
import { experimentalAllowed, experimentalState, normalizeTexture, textureAllowed } from './experimentalVisuals';

describe('experimental visuals policy', () => {
  const ready = { enabled: true, reduced: false, away: false, hidden: false, focused: true, contrast: false, transparency: false };
  it('validates saved texture levels and keeps ambient consent independent', () => {
    expect([null, '', 'invalid', 'on'].map(normalizeTexture)).toEqual(['full','full','full','full']);
    for (const level of ['full','quiet','off'] as const) expect(normalizeTexture(level)).toBe(level);
    expect(textureAllowed({...ready,texture:'off'})).toBe(false);
    expect(textureAllowed({...ready,texture:'quiet'})).toBe(true);
    expect(experimentalAllowed({...ready,texture:'off'})).toBe(true);
    expect(experimentalAllowed({...ready,texture:'full',enabled:false})).toBe(false);
    for (const key of ['reduced','contrast','transparency'] as const)
      expect(textureAllowed({...ready,texture:'quiet',[key]:true})).toBe(false);
  });
  it('runs only in a focused visible window with every accessibility gate open', () => {
    expect(experimentalAllowed(ready)).toBe(true);
    for (const key of ['reduced','away','hidden','contrast','transparency'] as const)
      expect(experimentalAllowed({...ready,[key]:true})).toBe(false);
    expect(experimentalAllowed({...ready,enabled:false})).toBe(false);
    expect(experimentalAllowed({...ready,focused:false})).toBe(false);
  });
  it('keeps idle still and prioritizes a real wait over working', () => {
    expect(experimentalState(false,false)).toBe('idle');
    expect(experimentalState(true,false)).toBe('working');
    expect(experimentalState(true,true)).toBe('waiting');
  });
  it('keeps static identity independent of the saved ambient-motion switch', () => {
    expect(textureAllowed({...ready, enabled:false})).toBe(true);
    for (const key of ['reduced','contrast','transparency'] as const)
      expect(textureAllowed({...ready,[key]:true})).toBe(false);
    // Static geometry schedules no work while hidden or unfocused.
    expect(textureAllowed({...ready,hidden:true,focused:false,away:true})).toBe(true);
  });
});
