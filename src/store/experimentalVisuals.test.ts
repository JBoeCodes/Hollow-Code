import { afterEach, expect, it, vi } from 'vitest';
import { createEffect, createRoot } from 'solid-js';
import { motionReduced } from '../motion';
// @ts-expect-error Solid's browser runtime is intentionally used for effects.
vi.mock('solid-js',()=>import('solid-js/dist/solid.js'));
afterEach(()=>vi.unstubAllGlobals());
it('preserves legacy motion consent while defaulting and persisting texture independently', async()=>{
  vi.resetModules();
  const saved = new Map<string,string>([['hollow.studio.experimentalVisuals','off']]);
  vi.stubGlobal('localStorage',{getItem:(k:string)=>saved.get(k)??null,setItem:(k:string,v:string)=>saved.set(k,v)});
  const dataset: Record<string,string> = {};
  vi.stubGlobal('document',{documentElement:{dataset}});
  const first = await import('./studio');
  expect(first.experimentalVisuals()).toBe(false);
  expect(first.studioTexture()).toBe('full');
  first.setStudioTexture('quiet');
  expect(saved.get('hollow.studio.texture')).toBe('quiet');
  expect(dataset.textureLevel).toBe('quiet');
  expect(saved.get('hollow.studio.experimentalVisuals')).toBe('off');
  vi.resetModules();
  const restored = await import('./studio');
  expect(restored.studioTexture()).toBe('quiet');
  restored.setStudioTexture('off');
  expect(restored.experimentalVisuals()).toBe(false);
  expect(saved.get('hollow.studio.texture')).toBe('off');
});
it('motion observers see the new document policy inside the preference update', async()=>{
  vi.resetModules();
  vi.stubGlobal('document',{documentElement:{dataset:{motion:'full'}}});
  vi.stubGlobal('localStorage',{getItem:()=>null,setItem:()=>{}});
  const studio=await import('./studio');
  let observed=false;
  const dispose=createRoot(dispose=>{
    createEffect(()=>{studio.studioMotion();observed=motionReduced();});
    return dispose;
  });
  await Promise.resolve();
  studio.setStudioMotion('reduced');
  expect(observed).toBe(true);
  dispose();
});
it('defaults on, persists explicit off, and starts inactive until focus is verified', async()=>{
  vi.resetModules();
  const saved = new Map<string,string>();
  vi.stubGlobal('localStorage',{getItem:(k:string)=>saved.get(k)??null,setItem:(k:string,v:string)=>saved.set(k,v)});
  vi.stubGlobal('document',{documentElement:{dataset:{}}});
  const first = await import('./studio');
  expect(first.experimentalVisuals()).toBe(true);
  expect(first.experimentalActive()).toBe(false);
  first.setExperimentalVisuals(false);
  expect(saved.get('hollow.studio.experimentalVisuals')).toBe('off');
  vi.resetModules();
  const restored = await import('./studio');
  expect(restored.experimentalVisuals()).toBe(false);
  restored.setExperimentalVisuals(true);
  expect(saved.get('hollow.studio.experimentalVisuals')).toBe('on');
});
