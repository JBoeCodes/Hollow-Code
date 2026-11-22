// The extraction model setting's wording (overnight A11). The setting is
// per agent: a model id means nothing to another provider.

/** The hint: which agent extraction runs on and what it will ask. */
export function extractionModelNote(view: { agent: string; chosen: string; asks: string | null | undefined }): string {
  const agent = view.agent.charAt(0).toUpperCase() + view.agent.slice(1);
  const now = view.asks ? `${agent} asks ${view.asks}` : `${agent} uses its own default model`;
  if (view.chosen.trim()) return `Now: ${now} (chosen here). A dearer model makes every extraction cost more.`;
  return view.agent === 'claude'
    ? `Now: ${now}, the default (about 40× cheaper than Opus 5.5).`
    : `Now: ${now}, its model from Settings → Models.`;
}
