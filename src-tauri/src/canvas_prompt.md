## Canvases

A canvas is one `hollow-canvas` fence holding one `<hc-canvas>` root and
nothing else:

```hollow-canvas
<hc-canvas id="release-status" title="Release status">
  <hc-stats>
    <hc-stat label="Checks passed" value="41" delta="+3" tone="success"></hc-stat>
    <hc-stat label="Open bugs" value="2" tone="warning"></hc-stat>
  </hc-stats>
  <hc-steps current="2">
    <div label="Merge" status="done"></div>
    <div label="Test bed">Running now</div>
    <div label="Install"></div>
  </hc-steps>
</hc-canvas>
```

- `id`: 1–64 lowercase letters, digits or hyphens. To
  revise a canvas from earlier in this conversation, emit the whole canvas
  again with the same `id`: Hollow keeps it as the next version and carries
  over how the person left it (open tab, sort). Use a new `id` for a
  different canvas. Never edit Hollow's saved copies.
- `title`: required plain text, at most 180 characters. Optional `height`:
  whole pixels from 80 to 1600, the space kept while it draws.
- The body is HTML, at most 128 KB. Nothing may come before the root or after
  `</hc-canvas>` inside the fence.
- Hollow saves every canvas (in a workspace under `outputs/canvases/`,
  elsewhere with the conversation). A broken one shows its source and lets
  the person ask you to fix it: answer with the corrected canvas, same `id`.

What a canvas can do (in Hollow's sandbox; no modes to pick):
- Your inline `<script>` and `<style>` run, including `<canvas>` drawing and
  custom elements. Write self-contained vanilla JS with no build step.
- There is no network: no fetch, no CDN, no remote images, fonts or
  stylesheets. Only data: and blob: images load. No forms, popups, workers,
  frames or plugins. Inline what you need; the kit below covers charts.
- No access to the app, Tauri IPC, cookies or localStorage. A link opens only
  through Hollow, after the person confirms the exact address.
- Anything remote is refused, and Hollow asks the person whether this
  canvas may reach the internet. Never rely on that: inline what you need.
- The kit follows Hollow's theme. For your own CSS use the `--app-*`
  variables (`--app-ink`, `--app-canvas`, `--app-surface`, `--app-primary`,
  `--app-muted`, `--app-hairline`, `--app-mono`); they update live, and
  script can redraw on the window's `hollow-theme` event. Keep the body
  background transparent.
- To keep a control as the person left it, call
  `hollowCanvas.set(key, value)` (JSON, 16 KB in all) from their input and
  read `hollowCanvas.get(key)` once `hollowCanvas.init` is set or the
  `hollow-init` event fires. The kit's tabs, table sort and details keep
  their own state.

A plain ```html or ```svg fence still renders, but it is not saved or
versioned, and outside workspaces and Chats its scripts stay off.
Prefer `hollow-canvas`.
