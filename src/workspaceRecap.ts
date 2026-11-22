// What remains of the workspace recap (the removed Overview section's model,
// COO rebuild L4): the context-health arithmetic the browser mock uses, and
// which output files are bookkeeping rather than Results.
import type { ContextDocHealth } from './workspaceTypes';
export { isResultBookkeeping } from './workspaceShell';

// ── Context health (durability D3) ──

/** The documents a turn's context carries and its 48 KB budget (Rust
 *  `workspaces::CONTEXT_DOCS`, `MAX_CONTEXT`). */
export const CONTEXT_DOCS = ['WORKSPACE.md', 'PLAN.md', 'SOURCES.md', 'AGENTS.md', 'NOTION.md'] as const;
export const CONTEXT_BUDGET = 48_000;

/** Rust `workspaces::context_health` for the browser mock: the smallest
 *  documents stay whole and the largest share what is left equally. */
export function contextHealthFromSizes(present: readonly { name: string; bytes: number }[], budget = CONTEXT_BUDGET): ContextDocHealth[] {
  const order = present.map((_, i) => i).sort((a, b) => present[a].bytes - present[b].bytes);
  const out: ContextDocHealth[] = present.map(p => ({ name: p.name, bytes: p.bytes, included: 0, limit: 0 }));
  let remaining = budget;
  order.forEach((i, done) => {
    const fair = Math.floor(remaining / (present.length - done));
    const share = Math.min(present[i].bytes, fair);
    out[i] = { ...out[i], included: share, limit: fair };
    remaining -= share;
  });
  return out;
}
