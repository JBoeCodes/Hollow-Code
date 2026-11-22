import type { CanvasItemSource, CanvasPlanItems } from '../generated/ipc';
import { mockCanvasCommand } from '../mockCanvases';

/** Reads a vault note by vault-relative path (the mock's workspace notes). */
type MockNoteReader = (path: string) => string | null | undefined;

/** Fixture implementation of the host's CAS contract. Never writes real files. */
export async function mockCanvasItems(command: string, args: Record<string, unknown>, read: MockNoteReader, write: (path: string, value: string) => void): Promise<unknown> {
  const error = localStorage.getItem('mockCanvasItemsReadError');
  if (error) throw error;
  const delay = Number(localStorage.getItem('mockCanvasItemsDelay') ?? 0);
  if (delay > 0) await new Promise(resolve => setTimeout(resolve, delay));
  const request = args.request as CanvasPlanItems | undefined;
  const workspaceId = request?.workspaceId ?? String(args.workspaceId);
  const id = request?.id ?? String(args.id);
  const canvas = await mockCanvasCommand('read_canvas', { workspaceId, id }) as CanvasItemSource['canvas'] | null;
  if (!canvas) throw 'The saved canvas is unavailable.';
  if (command === 'canvas_item_source') {
    return { canvas, project: null, projectUrl: null };
  }
  if (!request || canvas.revision !== request.revision) throw 'The saved canvas changed. Close and reopen Add items.';
  const path = `workspaces/${workspaceId}/PLAN.md`;
  const current = read(path);
  if (current == null) throw 'PLAN.md could not be read. Restore or create it in this workspace, then preview again.';
  const newline = current.includes('\r\n') ? '\r\n' : '\n';
  const heading = request.heading.trim();
  const lines = current.split(/(?<=\n)/);
  let offset = 0;
  const heads: { offset: number; level: number; text: string }[] = [];
  let fence = '';
  for (const line of lines) {
    const mark = /^\s*(`{3,}|~{3,})/.exec(line)?.[1] ?? '';
    if (mark) fence = fence && mark[0] === fence[0] && mark.length >= fence.length ? '' : fence || mark;
    const match = !fence && /^(#{1,6}) (.+?)\s*$/.exec(line);
    if (match) heads.push({ offset, level: match[1].length, text: match[2].replace(/\s+#+$/, '').trim() });
    offset += line.length;
  }
  const matches = heads.filter(h => h.text === heading);
  if (matches.length > 1) throw 'More than one Plan heading has that name. Choose a unique heading.';
  const found = matches[0];
  const at = found ? heads.find(h => h.offset > found.offset && h.level <= found.level)?.offset ?? current.length : current.length;
  const additions = request.rows.flatMap(([index, text]) => {
    const marker = `<!-- canvas-item:${id}:v${request.revision}:${index} -->`;
    if (current.includes(marker)) return [];
    const escaped = text.trim().replace(/[\\`*_\[\]<>#!]/g, '\\$&');
    return [`- [ ] ${escaped} (canvas ${id} v${request.revision}) ${marker}${newline}`];
  }).join('');
  const before = current.slice(0, at);
  const next = additions ? `${before}${before.endsWith(newline) ? newline : newline + newline}${found ? '' : `## ${heading}${newline}${newline}`}${additions}${newline}${current.slice(at)}` : current;
  if (request.expectedPreview == null) return next;
  if (request.expectedPreview !== next) throw 'Plan changed. Nothing was overwritten. Preview Plan again, then Confirm.';
  write(path, next);
  const writes = JSON.parse(localStorage.getItem('mockCanvasPlanWrites') ?? '[]') as string[];
  writes.push(next); localStorage.setItem('mockCanvasPlanWrites', JSON.stringify(writes));
  return next;
}
