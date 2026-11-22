import type { RunContextView } from '../types';

/** Settings, Context's one-line summary of a run's context manifest
 *  (agent effectiveness F11; Rust `run_manifest`). It names the run it
 *  describes: each run keeps its own manifest, so a second conversation
 *  preparing at the same time can no longer overwrite what is shown. */
export function contextReportLine(view: RunContextView): string {
  const m = view.manifest;
  const engines = [m.retrieval?.engine, m.connectors?.engine].filter((e): e is string => Boolean(e));
  const engine = engines.includes('jev-fallback')
    ? 'local, Jev unavailable'
    : engines.includes('jev') ? 'Jev'
    // S28: the conversation's last Jev connector decision, reused.
    : engines.includes('cached') ? 'Jev (reused)' : 'local';
  const candidates = m.retrieval?.candidates ?? [];
  const kept = candidates.filter((c) => c.kept).length;
  const decisions = m.connectors?.decisions ?? [];
  const attached = decisions.filter((d) => d.attach).length;
  const run = view.runId.length > 18 ? `${view.runId.slice(0, 18)}…` : view.runId;
  let line = `Last request (${engine}; run ${run}): kept ${kept} of ${candidates.length} passages`;
  if (decisions.length) line += `, attached ${attached} of ${decisions.length} “when relevant” connectors`;
  line += '.';
  const notes = m.fallbacks.filter((f) => f.stage !== 'session').map((f) => f.detail);
  if (notes.length) line += ` ${notes.join(' ')}`;
  return line;
}
