import { describe, expect, it } from 'vitest';
import { selectedAppIcon, appIconLabel } from './appIconPresets';
import type { CustomIcon } from './customIcons';
const saved = (preset?:string):CustomIcon => ({version:1,revision:'saved',preset,original:{format:'png',bytes:'retained'},editor:{mode:'make'},pngs:{}});
describe('app icon selection identity',()=>{
  it('only an absent override is the bundled default',()=>{
    expect(selectedAppIcon(null)).toBe('fold');
    expect(appIconLabel(null)).toBe('Fold');
    expect(selectedAppIcon(saved('fold'))).toBe('custom');
    expect(selectedAppIcon(saved('cut-glass'))).toBe('cut-glass');
    expect(appIconLabel(saved('cut-glass'))).toBe('Cut Glass');
  });
  it('keeps legacy and unrecognized artwork Custom',()=>{
    expect(appIconLabel(saved())).toBe('Custom');
    expect(appIconLabel(saved('unrecognized'))).toBe('Custom');
    expect(appIconLabel(saved('threshold'))).toBe('Threshold');
  });
});
