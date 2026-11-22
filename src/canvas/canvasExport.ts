/** Pure export choices shared by the dialog and mock; no store imports. */
import type { CanvasState } from '../canvasBridge';
export type ExportFormat = 'png' | 'pdf' | 'svg';
export type ExportPreset = 'email' | 'doc' | 'slide';
export type ExportLook = 'light' | 'dark' | 'match';
export interface CanvasExportRef { workspaceId: string; revision?: number; state?: CanvasState }
export interface CanvasExportSource { title: string; source: string; ref?: CanvasExportRef }
export interface CanvasExportRequest extends CanvasExportRef {
  source: string; format: ExportFormat; preset: ExportPreset; look: ExportLook;
  copy: boolean; target: 'file' | 'attachment'; scheme: 'light' | 'dark'; theme: Array<[string, string]>;
  svg?: string; flowIndex?: number;
  themeName?: string;
}
export interface ExportedFile {
  rel: string | null; path: string | null; pngBase64: string | null;
  width: number; height: number; scale: number; copied: boolean; warning: string | null;
}
export const EXPORT_WIDTHS = { email: 600, doc: 800, slide: 1280 } as const;
/** Repo ideas 2026-10-27 #4 (Rust `canvas_measure`): draw a reply canvas as
 *  its frame drew it, to measure its unresolved texts on the picture. */
export interface CanvasMeasureRequest {
  source: string; state: CanvasState | null; scheme: 'light' | 'dark'; theme: Array<[string, string]>;
  width: number; texture: boolean;
}
/** The four pictures (PNG, base64), text as it is, hidden, black and white,
 *  at `scale` device pixels per CSS px. */
export interface CanvasMeasureCaptures { shown: string; hidden: string; black: string; white: string; width: number; height: number; scale: number }
/** How one Measure on the picture pass draws its text (repo ideas 10-27 #4). */
export type MeasureText = 'shown' | 'hidden' | 'black' | 'white';
/** A capture child's job (Rust `CaptureJob`): an export, or since repo ideas
 *  10-27 #4 a Measure on the picture pass. */
export interface CaptureJobMeasure { text: MeasureText; texture: boolean }
export const EXPORT_LABEL = 'Export canvas…';
export function exportThemeSuffix(name?: string): string {
  return name ? '-' + (name.replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-+|-+$/g, '').toLowerCase().slice(0, 48).replace(/-+$/, '') || 'theme') : '';
}
export function exportName(id: string, revision: number, preset: ExportPreset, format: ExportFormat, dark: boolean, flowIndex?: number, themeName?: string): string {
  if (!/^[a-z0-9-]{1,64}$/.test(id) || !Number.isSafeInteger(revision) || revision < 1) throw new Error('Invalid canvas identity.');
  if (format === 'svg' && (!Number.isSafeInteger(flowIndex) || flowIndex! < 1)) throw new Error('Choose a flow.');
  return `${id}-v${revision}-${preset}${dark ? '-dark' : ''}${exportThemeSuffix(themeName)}${format === 'svg' ? `-flow-${flowIndex}` : ''}.${format}`;
}
