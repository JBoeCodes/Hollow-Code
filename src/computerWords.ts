// Words for the Computer card's action rows (Cua CU8); pure, so it is testable alone.
/** Words for one action row: "click, act, ok". */
export function actionWords(e: { tool: string; tier: string; outcome: string; detail: string; app?: string | null }): string {
  const what = e.outcome === 'ok' ? 'ran' : e.outcome === 'failed' ? 'failed' : `refused (${e.detail})`;
  return `${e.tool}${e.app ? ` in ${e.app}` : ''}: ${e.tier || 'no'} tier, ${what}`;
}
