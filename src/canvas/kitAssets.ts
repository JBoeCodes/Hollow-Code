/** The built Hollow Canvas Kit as strings, for inlining into artifact
 *  documents. Loaded lazily (`loadCanvasKit` in components/htmlSandbox.ts) so
 *  its bytes live in their own chunk instead of the launch-critical App chunk.
 *  The files are written by `scripts/build-canvas-kit.mjs`; never edit them. */
import KIT_JS from './kit/generated/kit.min.js?raw';
import KIT_CSS from './kit/generated/kit.min.css?raw';
import { KIT_VERSION } from './kit/manifest';

export interface CanvasKitAssets {
  version: string;
  js: string;
  css: string;
}

export const CANVAS_KIT: CanvasKitAssets = { version: KIT_VERSION, js: KIT_JS, css: KIT_CSS };
