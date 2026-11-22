/**
 * Launch restore (frontier F4). Hollow opens onto the conversation that was
 * on screen when it last closed, the way Codex and Conductor reopen the last
 * thread. Pure helpers; the store owns the reads and writes
 * (`restoreLastConversation` in store/workspaces.ts).
 */

/** localStorage key. `{ workspaceId, key }`; `key: null` is a new, unsent conversation. */
export const LAST_CONVERSATION_KEY = 'hollow.lastConversation';

export interface LastConversation { workspaceId: string; key: string | null }

export interface LaunchTarget {
  workspaceId: string;
  /** The conversation to reopen; absent means "the workspace's newest". */
  key?: string;
}

export function parseLastConversation(raw: string | null): LastConversation | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<LastConversation>;
    if (!v || typeof v.workspaceId !== 'string' || !v.workspaceId) return null;
    return { workspaceId: v.workspaceId, key: typeof v.key === 'string' && v.key ? v.key : null };
  } catch { return null; }
}

/** What to open at launch. A build older than F4 saved only the workspace
 *  (`hollow.workspace`), which reopens that workspace's newest conversation,
 *  as does a saved new conversation that was never sent (`key: null`). */
export function launchTarget(saved: LastConversation | null, legacyWorkspaceId: string | null): LaunchTarget | null {
  if (saved) return saved.key ? { workspaceId: saved.workspaceId, key: saved.key } : { workspaceId: saved.workspaceId };
  return legacyWorkspaceId ? { workspaceId: legacyWorkspaceId } : null;
}

/** A workspace's conversations for launch and reopen: its listed sessions,
 *  and its project main agent's, which isn't listed until its first turn
 *  (overnight 11-03 R4: a new project opens there, so a draft typed before
 *  that first turn was left behind for a blank new chat). */
export function workspaceConversationKeys(detail: { sessions: readonly { key: string }[]; projectAgentKey?: string | null }): string[] {
  const keys = detail.sessions.map(s => s.key);
  return detail.projectAgentKey && !keys.includes(detail.projectAgentKey) ? [...keys, detail.projectAgentKey] : keys;
}

/** Where launch lands, given the workspace's conversation keys (null when
 *  the workspace could not be read, e.g. it was deleted):
 *  - `open`: the saved conversation (or the workspace's newest) is there;
 *    the project main agent's counts archived or not yet sent
 *    (`workspaceConversationKeys`; the caller drops it once it moved to
 *    another project, `mainAgentMovedAway` in `store/workspaces.ts`);
 *  - `workspace`: the conversation was archived, removed or moved away, so
 *    its workspace opens on its newest conversation instead;
 *  - `home`: the workspace itself is gone. */
export function launchLanding(target: LaunchTarget, sessionKeys: readonly string[] | null): 'open' | 'workspace' | 'home' {
  if (!sessionKeys) return 'home';
  return !target.key || sessionKeys.includes(target.key) ? 'open' : 'workspace';
}
