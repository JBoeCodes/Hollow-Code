/** The built ask kit (Canvas C4 A1) as strings, for inlining into
 *  ask-canvases only (`<hc-canvas ask>`), after the display kit. Loaded lazily
 *  (`loadCanvasKitAsk` in components/htmlSandbox.ts) in its own chunk. The
 *  files are written by `scripts/build-canvas-kit.mjs`; never edit them. */
import KIT_ASK_JS from './kit/generated/kit-ask.min.js?raw';
import KIT_ASK_CSS from './kit/generated/kit-ask.min.css?raw';
import { KIT_VERSION } from './kit/manifest';
import type { CanvasKitAssets } from './kitAssets';

export const CANVAS_KIT_ASK: CanvasKitAssets = { version: KIT_VERSION, js: KIT_ASK_JS, css: KIT_ASK_CSS };
