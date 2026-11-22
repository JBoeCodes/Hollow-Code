/** Canvas C6 P1: the host's log of bridge messages it refused from its own
 *  frame. Each refusal becomes one record (`bridgeRefusal`: kind, size,
 *  reason, canvas id, conversation; never the payload), at most thirty a
 *  minute across this window's frames (`refusalLimiter`; the dropped count
 *  rides on the next record), written by the backend to a bounded app-data
 *  file (`canvas_bridge_log.rs`), never the vault. Fire and forget: a
 *  failed write changes nothing for the frame or the person. */
import { api } from '../api';
import { bridgeRefusal, isLoggedReason, refusalLimiter, type DraftRefusal, type Rejection } from '../canvasBridge';

const limiter = refusalLimiter();

export function logBridgeRefusal(data: unknown, reason: Rejection | DraftRefusal, canvasId: string | null, conversation: string | null): void {
  if (!isLoggedReason(reason)) return;
  const dropped = limiter.take(Date.now());
  if (dropped === null) return;
  const record = bridgeRefusal(data, reason, canvasId, conversation, dropped);
  if (record) void api.recordCanvasBridgeRefusal(record).catch(() => {});
}
