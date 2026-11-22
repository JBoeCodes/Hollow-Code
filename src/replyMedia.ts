// The videos under a finished reply (inline video, 2026-10-17): what the
// reply's receipt says it made or changed, then what its text names. Pure
// (`import type` erases), so the node vitest suite can load it.
import type { ExecutionReceipt } from './executionReceipt';
import { fileChips } from './turnFooter';
import { footerChipPath, isVideoPath, videoRefsFromText } from './chatFileLinks';

/** Players under one reply at most; the rest are listed as "+N more". */
export const REPLY_VIDEO_CAP = 3;

/**
 * The videos a receipt lists as created or changed, as absolute paths:
 * vault rows joined to `vaultRoot` (skipped while it is unknown), repository
 * rows to their root; deleted files and `clean` repo rows are left out. This
 * is how a video made through Bash (ffmpeg, a renderer) is found: the
 * receipt comes from History's before/after snapshot, whatever the agent.
 */
export function receiptVideos(receipt: ExecutionReceipt | undefined, vaultRoot: string | null): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const chip of fileChips(receipt, Infinity).shown) {
    if (chip.changeType === 'delete' || !isVideoPath(chip.path)) continue;
    const path = footerChipPath(chip, vaultRoot);
    if (path && !seen.has(path.toLowerCase())) { seen.add(path.toLowerCase()); out.push(path); }
  }
  return out;
}

/** Every video a reply made or names: receipt rows first, then the text's
 *  references (resolved against `base`), each once. */
export function replyVideos(
  receipt: ExecutionReceipt | undefined,
  text: string,
  base: string | null,
  vaultRoot: string | null,
): string[] {
  const out = receiptVideos(receipt, vaultRoot);
  const seen = new Set(out.map(p => p.toLowerCase()));
  for (const path of videoRefsFromText(text, base)) if (!seen.has(path.toLowerCase())) { seen.add(path.toLowerCase()); out.push(path); }
  return out;
}

/** The players to show and the rest, listed as "+N more videos". */
export function splitReplyVideos(paths: string[], cap = REPLY_VIDEO_CAP): { shown: string[]; more: string[] } {
  return { shown: paths.slice(0, cap), more: paths.slice(cap) };
}
