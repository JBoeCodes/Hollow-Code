/** Recorded agent identities; unknown providers keep their own label. */
const AGENT: Record<string, string> = { claude: 'Claude', codex: 'Codex', grok: 'Grok', antigravity: 'Antigravity' };
export const agentLabel = (agent: string | null | undefined) => agent ? AGENT[agent.toLowerCase()] ?? agent : null;
