import { createSignal } from 'solid-js';
import { normalizeTextScale, type TextScale } from './textScale';
import { normalizeTexture, type TextureLevel } from '../experimentalVisuals';
export { TEXT_SCALES, TEXT_SCALE_OPTIONS, type TextScale } from './textScale';

function stored(key: string) { try { return localStorage.getItem(key); } catch { return null; } }
function save(key: string, value: string) { try { localStorage.setItem(key, value); } catch { /* session preference still works */ } }
// Workspace pins moved into the briefs (store/workspacePins.ts, 2026-10-21).
export type MotionMode = 'full' | 'reduced';
const MOTION_KEY = 'hollow.studio.motion';
export const [studioMotion, setMotion] = createSignal<MotionMode>(stored(MOTION_KEY) === 'reduced' ? 'reduced' : 'full');
document.documentElement.dataset.motion = studioMotion();
export function setStudioMotion(mode: MotionMode) {
  document.documentElement.dataset.motion = mode; setMotion(mode); save(MOTION_KEY, mode);
}

const EXPERIMENTAL_KEY = 'hollow.studio.experimentalVisuals';
// Keep the existing key: old Off remains Off for ambient motion.
export const [experimentalVisuals, writeExperimental] = createSignal(stored(EXPERIMENTAL_KEY) !== 'off');
/** False until the lazy controller verifies focus, motion and accessibility. */
export const [experimentalActive, setExperimentalActive] = createSignal(false);
export function setExperimentalVisuals(enabled: boolean) {
  writeExperimental(enabled); save(EXPERIMENTAL_KEY, enabled ? 'on' : 'off');
}

const TEXTURE_KEY = 'hollow.studio.texture';
export const [studioTexture, writeTexture] = createSignal<TextureLevel>(normalizeTexture(stored(TEXTURE_KEY)));
document.documentElement.dataset.textureLevel = studioTexture();
export function setStudioTexture(value: TextureLevel) {
  const level = normalizeTexture(value);
  document.documentElement.dataset.textureLevel = level;
  writeTexture(level); save(TEXTURE_KEY, level);
}

/** Density and text scale (next-level P01). Both are presentation only:
 *  the same components at tighter spacing, and every type step multiplied.
 *  main.tsx mirrors them onto <html> before first paint. */
export type DensityMode = 'comfortable' | 'compact';
const DENSITY_KEY = 'hollow.studio.density';
export const [studioDensity, setDensitySignal] = createSignal<DensityMode>(stored(DENSITY_KEY) === 'compact' ? 'compact' : 'comfortable');
document.documentElement.dataset.density = studioDensity();
export function setStudioDensity(mode: DensityMode) {
  setDensitySignal(mode); save(DENSITY_KEY, mode); document.documentElement.dataset.density = mode;
}
const SCALE_KEY = 'hollow.studio.textScale';
const savedScale = stored(SCALE_KEY);
export const [studioTextScale, setScaleSignal] = createSignal<TextScale>(normalizeTextScale(savedScale));
document.documentElement.dataset.textScale = studioTextScale();
export function setStudioTextScale(scale: TextScale) {
  scale = normalizeTextScale(scale);
  setScaleSignal(scale); save(SCALE_KEY, scale); document.documentElement.dataset.textScale = scale;
}
