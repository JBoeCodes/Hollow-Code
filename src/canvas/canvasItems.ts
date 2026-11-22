import { readAsText, type TextBlock } from './canvasText';

export interface CanvasItem { index: number; text: string; selected: boolean }
export const ITEM_LIMIT = 50;
export function itemRows(blocks: TextBlock[]): { rows: CanvasItem[]; omitted: number } {
  const texts = blocks.flatMap(b => b.kind === 'list' ? b.items : b.kind === 'table' ? b.rows.map(r => r.join(' · ')) : []);
  const rows = texts.map((text, index) => ({ index, text: text.replace(/^(?:Not done|Done):\s*/, '').replace(/\s+/g, ' ').trim(), selected: true })).filter(r => r.text);
  return { rows: rows.slice(0, ITEM_LIMIT), omitted: Math.max(0, rows.length - ITEM_LIMIT) };
}
export const savedItems = (html: string) => itemRows(readAsText(html));
export const itemKey = (workspace: string, id: string, revision: number, index: number) =>
  JSON.stringify(['canvas-item', workspace, id, revision, index]);
export const itemOrigin = (id: string, revision: number) => `canvas:${id}:v${revision}`;
