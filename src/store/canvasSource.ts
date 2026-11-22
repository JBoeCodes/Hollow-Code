import type { Canvas } from '../generated/ipc';
import { openSessionByKey } from './workspaces';

/** A version's recorded source, never a title search or the latest chat. */
export function canvasSourceSession(canvas: Canvas, revision = canvas.revision): string | null {
  return canvas.revisions.find(r => r.revision === revision)?.session ??
    (revision === canvas.revision ? canvas.session : null) ?? canvas.sessionKey;
}

export async function openCanvasSource(canvas: Canvas, revision = canvas.revision): Promise<void> {
  const key = canvasSourceSession(canvas, revision);
  if (!key) throw Error('This version has no source conversation recorded.');
  if (!await openSessionByKey(key)) throw Error('The source conversation could not be opened.');
}
