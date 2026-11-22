import { defineConfig } from 'vitest/config';

// Frontend unit tests for the pure client-side modules (utils, taskQuery,
// dragdrop helpers). No DOM, no Tauri — anything importing store.ts or
// @tauri-apps/api stays out of scope here (those run in `cargo test` /
// the real app). Deliberately separate from vite.config.ts so the test
// runner never pulls in vite-plugin-solid or the dev-server config.
export default defineConfig({
  test: {
    include: ['src/**/*.test.ts'],
    environment: 'node',
    // CI sets CI=true, where Vitest refuses a stray `.only`; locally it would
    // quietly skip the rest of the file. Refuse it everywhere (overnight S2).
    allowOnly: false,
    // Vitest stubs every CSS import to '' unless it is listed here, even with
    // `?raw`. These are read as text: the Canvas Kit stylesheets that
    // artifact documents inline (the kit's and, C4, the ask kit's), and the
    // token file the theme export covers, and the two stylesheets whose
    // motion gates a test reads (session indicator, Main agent space), and
    // foundations, whose `.canvas` overflow rule a test pins (10-11 fix),
    // and the side pane, whose compact tab dot a test pins (P1-08), and the
    // self-hosted start chip, whose motion gate a test reads (Q6), the shell
    // and workbench stylesheets, and the chat stylesheet, whose long-message
    // fold a test pins (C3 follow-up).
    css: { include: [/src\/canvas\/kit\/generated\/kit(-ask)?\.min\.css/, /src\/styles\/(tokens|session-indicator|main-space|foundations|side-pane|self-hosted-status|shell-workspace|workbench|chat-workspace|tracker-tasks|motion-kit|build-changes)\.css/] },
  },
});
