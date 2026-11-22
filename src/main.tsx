/* @refresh reload */
import { render } from 'solid-js/web';
// Self-hosted fonts (no CDN/network dependency). Imported before
// styles.css so the @font-face rules are defined when the tokens
// reference them. Inter carries UI, prose, and headings;
// JetBrains Mono carries code/data.
// The opsz build adds Inter's optical-size axis (14-32). Paired with
// `font-optical-sizing: auto` in foundations.css, display sizes get their
// intended tighter shapes; a no-op at the 11-13px metadata sizes.
import '@fontsource-variable/inter/opsz.css';
import '@fontsource/jetbrains-mono/400.css';
import '@fontsource/jetbrains-mono/500.css';
import '@fontsource/jetbrains-mono/600.css';
import './styles.css';
import { MOCK_BUILD, isBrowserMockMode } from './mockMode';
import { normalizeTextScale } from './store/textScale';
import { normalizeTexture } from './experimentalVisuals';

// Theme bootstrap for EVERY window: the storeless windows
// (md-preview, canvas-capture) never load ./store, so
// without this they'd render the stock-light tokens forever. The main
// window's store re-applies (and may layer a custom theme) right after.
try {
  document.documentElement.dataset.motion = localStorage.getItem('hollow.studio.motion') === 'reduced' ? 'reduced' : 'full';
  document.documentElement.dataset.density = localStorage.getItem('hollow.studio.density') === 'compact' ? 'compact' : 'comfortable';
  document.documentElement.dataset.textScale = normalizeTextScale(localStorage.getItem('hollow.studio.textScale'));
  document.documentElement.dataset.textureLevel = normalizeTexture(localStorage.getItem('hollow.studio.texture'));
  const saved = localStorage.getItem('theme');
  document.documentElement.dataset.theme =
    saved === 'light' || saved === 'dark'
      ? saved
      : window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  // Storeless windows follow the system too. The main store owns its own signal.
  if (saved !== 'light' && saved !== 'dark') {
    window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener('change', event => {
      const preference = localStorage.getItem('theme');
      if (preference !== 'light' && preference !== 'dark') document.documentElement.dataset.theme = event.matches ? 'dark' : 'light';
    });
  }
} catch { /* default tokens apply */ }

// Other windows open with ?window=… and dynamic-import their own bundle so
// they never evaluate `./App` / `./store` / their side effects (boot-time
// vault + M365 fetches, signal initialisation, etc.), which expect the main
// window's broader capability set. The quick-capture and quick-ingest
// overlays were removed in S5 (2026-11-03): their keys open the main
// window's Add sheet.
//
// The main window imports `./App` directly because it's the common
// case and the user lands on it immediately; lazy-loading would add
// a perceptible flash.
const params = new URLSearchParams(window.location.search);

/// How long the cold-boot cascade runs. The single source of truth for it:
/// `bootApp` writes it to `--boot-ms` so the CSS in foundations.css times its
/// entrance off the same number, and strips `.is-booting` when it elapses.
/// Two independent clocks is what made the launch animation unreliable.
const BOOT_MS = 420;

if (MOCK_BUILD && params.get('window') === 'smoke-artifact' && isBrowserMockMode()) {
  import('./SmokeHarness').then(m => {
    render(() => <m.SmokeHarness />, document.getElementById('root')!);
  });
} else if (params.get('window') === 'canvas-capture') {
  import('./canvas/CaptureHost').then(m => render(() => <m.CaptureHost />, document.getElementById('root')!));
} else if (params.get('window') === 'md-preview') {
  // Markdown preview windows (one per OS-opened .md file). Storeless like the
  // overlays — renders the file + a "Save to vault" action, never loads `./App`.
  import('./MdPreview').then(m => {
    render(() => <m.MdPreview />, document.getElementById('root')!);
  });
} else {
  // Overlay title bar (MAIN WINDOW ONLY). tauri.conf.json gives the main
  // window `titleBarStyle: "Overlay"`: no native title strip, and the macOS
  // traffic lights float over the top-left of the web content. The shell
  // leaves room for them only when they are really there; `?chrome=overlay`
  // lets a browser capture preview the same layout.
  try {
    const tauri = '__TAURI_INTERNALS__' in window && !isBrowserMockMode();
    if ((tauri && /Mac/.test(navigator.userAgent)) || params.get('chrome') === 'overlay') {
      document.documentElement.dataset.chrome = 'overlay';
    }
  } catch { /* plain chrome */ }

  // Liquid Glass overlay (MAIN WINDOW ONLY — the storeless overlays keep
  // their own transparent-window styling and must never get data-glass).
  // Mirror the persisted glass state onto <html data-glass> at pre-render so
  // the first paint is correct, and fire the native-vibrancy invoke as early
  // as possible — before ./App and ./store even load — so the desktop blur is
  // in place before the window becomes visible (~150ms deferred show). Without
  // this the always-transparent window would briefly flash the raw, unblurred
  // desktop through the translucent glass surfaces.
  try {
    if (localStorage.getItem('glass') === 'on') {
      document.documentElement.setAttribute('data-glass', 'on');
      const dark = document.documentElement.dataset.theme === 'dark';
      import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke('set_window_vibrancy', { enabled: true, dark }))
        .catch(() => { /* non-Tauri dev / not ready — store boot re-applies */ });
    }
  } catch { /* ignore */ }

  // Main window. One cheap IPC round-trip decides the root: when no
  // config.json could be loaded the backend booted in setup mode, and we
  // render the storeless Onboarding wizard INSTEAD of the shell — importing
  // ./App would fire the store's boot-time vault fetches against a vault
  // that doesn't exist yet. Any failure to answer (e.g. plain-browser dev
  // smoke without Tauri) falls through to the normal shell.
  const bootApp = () => {
    // Mark the document as "booting" so the cold-boot cascade animations in
    // styles.css fire on initial mount; stripping the class afterwards lets
    // normal page-enter take over. Reduced-motion bypasses entirely.
    const reduced = document.documentElement.dataset.motion === 'reduced' || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    const root = document.documentElement;
    if (!reduced) {
      // One source of truth for the boot window: JS strips the class and CSS
      // times the cascade off the same number.
      root.style.setProperty('--boot-ms', `${BOOT_MS}ms`);
      root.classList.add('is-booting');
    }
    import('./App').then(m => {
      render(() => <m.App />, document.getElementById('root')!);
      // H6: the first-run setup sheet, its own chunk and root, after the
      // shell has painted. Only an open `setup` block in config.json shows it.
      setTimeout(() => { void import('./firstRunMount').then(f => f.checkFirstRun()); }, 400);
      if (reduced) return;
      // Anchor the strip on actual first paint, not on module evaluation.
      // The cascade cannot start until this render lands, and `import('./App')`
      // is the app's largest chunk — so a timer armed before the import spent
      // most of its budget loading, which made the launch animation depend on
      // whether the chunk was warm in disk cache.
      requestAnimationFrame(() => requestAnimationFrame(() => {
        setTimeout(() => root.classList.remove('is-booting'), BOOT_MS + 80);
      }));
    });
  };
  // Named whole-app scenarios on top of a fixture (`&scenario=interrupted`;
  // docs/FIXTURES.md), applied after the demo seed so they can change it.
  const applyMockScenarios = async () => {
    if (!MOCK_BUILD || !params.get('scenario')) return;
    (await import('./mockScenarios')).applyScenarios(params.get('scenario'));
  };
  // `MOCK_BUILD` first so a production build drops this branch and the
  // `mockDemo` fixtures it imports (see mockMode.ts).
  if (MOCK_BUILD && isBrowserMockMode()) {
    // `setup=1` gives browser smoke/manual QA the same storeless first-run
    // branch as an unconfigured Tauri install. Normal browser mock sessions
    // still boot the full shell.
    if (new URLSearchParams(window.location.search).get('setup') === '1') {
      import('./Onboarding').then(m => {
        render(() => <m.OnboardingRoot />, document.getElementById('root')!);
      });
    } else if (params.get('demo')) {
      // Realistic design-review / scale fixtures (next-level P01/P21).
      import('./mockDemo').then(async m => { m.seedDemo(params.get('demo')); await applyMockScenarios(); bootApp(); });
    } else {
      void applyMockScenarios().then(bootApp);
    }
  } else {
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<{ configured: boolean }>('setup_status'))
      .then(s => {
        if (s && !s.configured) {
          import('./Onboarding').then(m => {
            render(() => <m.OnboardingRoot />, document.getElementById('root')!);
          });
        } else {
          bootApp();
        }
      })
      .catch(bootApp);
  }
}
