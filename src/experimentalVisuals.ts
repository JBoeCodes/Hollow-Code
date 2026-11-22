export type TextureLevel = 'full' | 'quiet' | 'off';
export function normalizeTexture(value: unknown): TextureLevel {
  return value === 'quiet' || value === 'off' ? value : 'full';
}
export type VisualPolicy = {enabled:boolean; reduced:boolean; away:boolean; hidden:boolean; focused:boolean; contrast:boolean; transparency:boolean; texture?:TextureLevel};
/** Static identity is default-on, independent of the ambient-motion switch. */
export function textureAllowed(s: VisualPolicy) {
  return s.texture !== 'off' && !s.reduced && !s.contrast && !s.transparency;
}
/** Ambient work additionally requires a focused visible window and consent. */
export function experimentalAllowed(s: VisualPolicy) {
  return s.enabled && textureAllowed({...s, texture:'full'}) && !s.away && !s.hidden && s.focused;
}
export function experimentalState(working: boolean, waiting: boolean) {
  return waiting ? 'waiting' : working ? 'working' : 'idle';
}
