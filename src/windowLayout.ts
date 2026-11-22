// Live window tiers shared by root-container CSS and navigation/pane policy.
// No store imports: chatRuntime reads this during module initialization.
export function windowAtMost(tier: 'compact' | 'medium'): boolean {
  if (typeof window === 'undefined') return false;
  const root = typeof document === 'undefined' ? null : document.documentElement;
  const style = root ? getComputedStyle(root) : null;
  const em = Number(style?.getPropertyValue(`--window-${tier}-em`)) || (tier === 'compact' ? 53.75 : 68.75);
  const font = parseFloat(style?.fontSize ?? '') || 16;
  return (root?.clientWidth || window.innerWidth) <= em * font;
}
