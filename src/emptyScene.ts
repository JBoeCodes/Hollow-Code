export type EmptyScene = 'inbox' | 'commitments' | 'results' | 'sources' | 'mission' | 'search' | 'waiting' | 'files' | 'visuals' | 'tasks' | 'automations';
export type EmptyEvidence = { ready: boolean; failed?: boolean; partial?: boolean; filtered?: boolean };
/** Decoration follows the same evidence as the words. Never celebrate missing coverage. */
export function emptyScene(kind: EmptyScene, evidence: EmptyEvidence): EmptyScene | null {
  if (!evidence.ready || evidence.failed || evidence.partial) return null;
  return evidence.filtered ? 'search' : kind;
}
/** Generic empty states keep their existing icon for filtered results. */
export function emptyStateScene(kind: EmptyScene, evidence: EmptyEvidence): EmptyScene | null {
  return evidence.filtered ? null : emptyScene(kind, evidence);
}
export function recordingsMissing(state: 'ok' | 'notConnected' | 'unavailable' | undefined, filter: string): boolean { return state === 'unavailable' && (filter === 'all' || filter === 'recording'); }
