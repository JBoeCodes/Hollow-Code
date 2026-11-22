import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

export default defineConfig({
  // The What's new page was cut (overnight 11-08 S4). docs/WHATS_NEW.md
  // stays the user log. scripts/whats-new-split.mjs is no longer a plugin.
  plugins: [solid()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Scratch checkouts can contain their own tsconfigs/HTML. Watching them
    // causes unrelated full reloads (including while macOS hydrates files).
    watch: { ignored: ['**/src-tauri/**', '**/legacy/**', '**/.context/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: 'esnext',
    minify: 'esbuild',
    sourcemap: false,
    chunkSizeWarningLimit: 1500,
  },
});
