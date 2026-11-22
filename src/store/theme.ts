import { createResource, createSignal } from 'solid-js';
import { motionReduced } from '../motion';
import { api } from '../api';
import type { CustomTheme, CustomThemeMeta } from '../types';

// ── Theme
//
// The app has three theme modes:
// - 'light' / 'dark' — the built-in palettes baked into `styles.css`
//   via `[data-theme=...]` selectors.
// - a custom theme — a kitty-style palette file under
//   `~/Library/Application Support/Hollow/themes/`. The frontend parses
//   it (well — Rust parses it; we just receive a `CustomTheme`), derives
//   the full set of design tokens, and applies them as inline CSS
//   variables on `<html>` so they win over the baked-in palette.
//
// `theme()` always reports `'light' | 'dark'` so legacy code (SVG color
// pickers in Graph.tsx, etc.) keeps working — when a custom theme is
// active, `theme()` reflects the theme's `isDark` field.
export type Theme = 'light' | 'dark';

const ACTIVE_CUSTOM_KEY = 'activeCustomTheme';
const systemAppearance = window.matchMedia?.('(prefers-color-scheme: dark)');
const systemTheme = (): Theme => systemAppearance?.matches ? 'dark' : 'light';
const savedPreference = (() => { try { return localStorage.getItem('theme'); } catch { return null; } })();
export const [matchSystem, setMatchSystem] = createSignal(savedPreference !== 'light' && savedPreference !== 'dark');
let selection = 0;
export const themeSelection = () => selection;
/** Reserve a choice before an asynchronous palette read begins. */
export const beginThemeSelection = () => ++selection;

const initialTheme: Theme = (() => {
  try {
    const saved = localStorage.getItem('theme');
    if (saved === 'light' || saved === 'dark') return saved;
  } catch { /* ignore */ }
  return systemTheme();
})();
export const [theme, setTheme] = createSignal<Theme>(initialTheme);
document.documentElement.dataset.theme = initialTheme;
systemAppearance?.addEventListener('change', () => {
  if (!matchSystem()) return;
  const base = systemTheme();
  setTheme(base);
  document.documentElement.dataset.theme = base;
  api.setWindowVibrancy(glass(), base === 'dark').catch(() => {});
});

/// When non-null, `customTheme()` overrides the baked palette with
/// inline `--canvas` / `--ink` / … variables on the document element.
/// Stays null in stock light/dark mode.
export const [customTheme, _setCustomTheme] = createSignal<CustomTheme | null>(null);

/// Rehydrate the previously-active custom theme on boot. Failures (file
/// renamed/deleted out from under us, Tauri not ready, etc.) silently
/// drop back to the saved light/dark mode and clear the stale pointer.
(() => {
  let savedName: string | null = null;
  try { savedName = localStorage.getItem(ACTIVE_CUSTOM_KEY); } catch { /* ignore */ }
  if (!savedName || matchSystem()) return;
  const owner = selection;
  api.readCustomTheme(savedName).then((t) => {
    if (owner === selection) return applyCustomTheme(t, { persist: false });
  }).catch(() => {
    if (owner === selection) {
      try { localStorage.removeItem(ACTIVE_CUSTOM_KEY); } catch { /* ignore */ }
    }
  });
})();

// ── Liquid Glass — a structural overlay, orthogonal to light/dark.
//
// Glass adds translucency + native macOS window vibrancy on top of either
// base polarity. It lives in its OWN `data-glass` attribute + signal so the
// existing `[data-theme="dark"]` rules and every `theme()` consumer (Graph
// color pickers, etc.) are untouched — `theme()` still reports the base
// 'light'/'dark'. The two "Liquid Glass" Appearance cards set the base via
// `setActiveTheme({ base, glass: true })`, which flips this on; picking a
// stock or custom theme flips it off (custom themes paint opaque inline
// surfaces that are incompatible with glass).
export const [glass, setGlass] = createSignal<boolean>((() => {
  try { return localStorage.getItem('glass') === 'on'; } catch { return false; }
})());

function applyGlassAttr(on: boolean) {
  const el = document.documentElement;
  if (on) el.setAttribute('data-glass', 'on');
  else el.removeAttribute('data-glass');
  try { localStorage.setItem('glass', on ? 'on' : 'off'); } catch { /* ignore */ }
}

// Boot: reflect persisted glass into the attribute + native vibrancy.
// (main.tsx fires the vibrancy invoke even earlier — before this module
// loads — so the desktop blur is in place before the window shows and no
// raw-desktop flash leaks through the transparent window; this is the
// store-side backstop and keeps the attribute authoritative.)
applyGlassAttr(glass());
if (glass()) {
  api.setWindowVibrancy(true, initialTheme === 'dark').catch(() => { /* ignore */ });
}

/// Every Hollow design token that the custom-theme deriver writes to.
/// Used to wipe the inline overrides cleanly when the user goes back to
/// stock light/dark — `removeProperty()` is selective so we don't blow
/// away other inline styles set elsewhere.
const CUSTOM_VAR_KEYS = [
  '--canvas', '--surface-soft', '--surface-card', '--surface-cream-strong',
  '--surface-raised',
  '--hairline', '--hairline-soft',
  '--ink', '--body-strong', '--body', '--muted', '--muted-soft',
  '--primary', '--primary-active', '--primary-disabled',
  '--accent-teal', '--accent-amber',
  '--on-primary', '--on-error',
  '--success', '--warning', '--error', '--risk',
  '--primary-soft', '--primary-soft-strong',
  '--teal-soft', '--amber-soft', '--success-soft', '--error-soft',
  '--risk-soft',
  // Syntax colours come from the palette's ANSI slots when it has them.
  '--syn-keyword', '--syn-string', '--syn-comment', '--syn-number',
  '--syn-function', '--syn-type', '--syn-builtin', '--syn-operator',
  '--syn-punct', '--syn-property', '--syn-tag', '--syn-attr', '--syn-meta',
];

function clearCustomVars() {
  const ds = document.documentElement.style;
  for (const k of CUSTOM_VAR_KEYS) ds.removeProperty(k);
}

/// Apply a parsed custom theme: align light/dark (so baked-in shadow +
/// selection rules pick the right side), set every derived CSS variable
/// inline, and persist the name unless the caller is just rehydrating.
export async function applyCustomTheme(t: CustomTheme, opts?: { persist?: boolean }) {
  const owner = ++selection;
  const { deriveCustomVars } = await import('../customThemeVars');
  if (owner !== selection) return;
  const persist = opts?.persist !== false;
  const baseMode: Theme = t.isDark ? 'dark' : 'light';
  setMatchSystem(false);
  setTheme(baseMode);
  document.documentElement.dataset.theme = baseMode;
  if (persist) { try { localStorage.setItem('theme', baseMode); } catch { /* ignore */ } }

  const derived = deriveCustomVars(t);
  const ds = document.documentElement.style;
  for (const k of Object.keys(derived)) ds.setProperty(k, derived[k]);
  _setCustomTheme(t);
  if (persist) {
    try { localStorage.setItem(ACTIVE_CUSTOM_KEY, t.name); } catch { /* ignore */ }
  }
}

/// Drop the custom-theme override, revert to the saved light/dark
/// palette. Idempotent.
export function clearCustomThemeOverride() {
  ++selection;
  clearCustomVars();
  _setCustomTheme(null);
  try { localStorage.removeItem(ACTIVE_CUSTOM_KEY); } catch { /* ignore */ }
}

/// Wrap any theme mutation in the radial "ink reveal" sweep. Snapshots
/// the current canvas color onto a clip-path overlay, runs `apply`, then
/// peels the overlay away from the click origin. Falls back to a plain
/// swap when either motion preference requests reduction.
function revealThemeChange(apply: () => void, ev?: { clientX: number; clientY: number }) {
  if (motionReduced()) { apply(); return; }

  const origin = ev ?? { clientX: window.innerWidth / 2, clientY: window.innerHeight / 2 };
  const oldCanvas = getComputedStyle(document.documentElement)
    .getPropertyValue('--canvas').trim() || '#f4ecd8';

  const overlay = document.createElement('div');
  overlay.className = 'theme-flash';
  overlay.style.background = oldCanvas;
  const w = window.innerWidth;
  const h = window.innerHeight;
  const dx = Math.max(origin.clientX, w - origin.clientX);
  const dy = Math.max(origin.clientY, h - origin.clientY);
  const reach = Math.hypot(dx, dy);
  overlay.style.setProperty('--reveal-x', `${origin.clientX}px`);
  overlay.style.setProperty('--reveal-y', `${origin.clientY}px`);
  overlay.style.setProperty('--reveal-r', `${reach}px`);
  document.body.appendChild(overlay);
  // Force a layout flush so the starting clip-path is committed before
  // we add the animating class — otherwise some browsers collapse the
  // two states and skip the animation.
  void overlay.offsetWidth;

  apply();

  overlay.classList.add('is-revealing');
  overlay.addEventListener('animationend', () => overlay.remove(), { once: true });
  // Safety: yank the overlay if the animation fails to fire for any
  // reason (HMR mid-toggle, etc.). 1.5× the animation duration.
  setTimeout(() => overlay.remove(), 900);
}

/// Switch to a stock palette (`'light' | 'dark'`) or a parsed custom
/// theme. Wraps the change in the radial-reveal sweep anchored at the
/// click origin.
export function setActiveTheme(
  spec: Theme | 'system' | { custom: CustomTheme } | { base: Theme; glass: true },
  ev?: { clientX: number; clientY: number },
) {
  let pending: Promise<void> | undefined;
  ++selection;
  revealThemeChange(() => {
    if (typeof spec === 'string') {
      clearCustomThemeOverride();
      setMatchSystem(spec === 'system');
      const base = spec === 'system' ? systemTheme() : spec;
      setTheme(base);
      document.documentElement.dataset.theme = base;
      try { localStorage.setItem('theme', spec); } catch { /* ignore */ }
      // Leaving glass for a stock palette → drop the overlay + native vibrancy.
      if (glass()) {
        setGlass(false);
        applyGlassAttr(false);
      }
      api.setWindowVibrancy(false, base === 'dark').catch(() => { /* ignore */ });
    } else if ('custom' in spec) {
      // Custom themes set opaque inline surfaces — incompatible with glass.
      if (glass()) {
        setGlass(false);
        applyGlassAttr(false);
        api.setWindowVibrancy(false, spec.custom.isDark).catch(() => { /* ignore */ });
      }
      pending = applyCustomTheme(spec.custom);
    } else {
      // Liquid Glass: a base light/dark polarity + the translucency overlay
      // + the native vibrancy layer behind the transparent window.
      clearCustomThemeOverride();
      setMatchSystem(false);
      setTheme(spec.base);
      document.documentElement.dataset.theme = spec.base;
      try { localStorage.setItem('theme', spec.base); } catch { /* ignore */ }
      setGlass(true);
      applyGlassAttr(true);
      api.setWindowVibrancy(true, spec.base === 'dark').catch(() => { /* ignore */ });
    }
  }, ev);
  return pending;
}

// ── Custom theme color derivation ─────────────────────────────────────
//
// The kitty palette gives us 4 anchor colors (accent, fg, bg, +16-slot
// ANSI). Hollow uses ~32 design tokens. We derive the rest by mixing
// fg/bg in fixed ratios for surface levels, picking semantic colors
// from the ANSI palette (1=red, 2=green, 6=cyan…), and synthesizing
// alpha tints from the accent.
//
// `dark` themes get slightly stronger surface mixes because the
// fg-on-bg contrast is higher — a 4% mix that reads "tinted" on cream
// disappears on warm black. Same logic for the soft-tint alphas.


// ── Custom theme resource ─────────────────────────────────────────────
//
// The Settings picker uses this; Settings.tsx triggers a refetch after
// any write/delete. Out-of-band edits (user edits a `.theme` file in
// their text editor while the app is open) are picked up the next time
// Settings is opened.
export const [customThemes, { refetch: refetchCustomThemes }] =
  createResource<CustomThemeMeta[]>(async () => {
    try { return await api.listCustomThemes(); } catch { return []; }
  });

