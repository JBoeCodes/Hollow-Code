// The one piece of claudeAccounts.ts the chat needs at launch, in its own
// module so the rest of it (Settings, Claude accounts) stays in the lazy
// Settings chunk instead of the App chunk (P1-42's App-chunk trim).

/** R1: the quiet chat notice of an `accountSwitched` event, when a turn's
 *  Claude account ran out and the same conversation resumed on another:
 *  "Switched to Tech (Admin hit its limit)". Turn guards (2026-10-09): a
 *  running turn moved at the Spread ceiling says "passed its ceiling".
 *  Labels come from the backend. */
export function accountSwitchNotice(from: string, to: string, reason?: string | null): string {
  const next = to.trim() || 'another account';
  const previous = from.trim();
  const why = reason === 'ceiling' ? 'passed its ceiling' : 'hit its limit';
  return previous ? `Switched to ${next} (${previous} ${why})` : `Switched to ${next}`;
}

/** Repo ideas 2026-10-27 #8: the chat notice of an `agentContinued` event,
 *  when a turn ran out of usage having changed nothing and the person chose
 *  another agent for that (Settings, Models, When an agent runs
 *  out): "Claude ran out · continuing on Codex". Labels come from the
 *  backend. */
export function agentContinuedNotice(from: string, to: string): string {
  return `${from.trim() || 'The agent'} ran out · continuing on ${to.trim() || 'another agent'} (Settings → Models → When an agent runs out)`;
}

/** The message that continuation sends as yours (Rust
 *  `agent_out::continue_prompt`): "<agent> ran out of usage before it could
 *  answer my last message…, and it changed nothing. Please continue that
 *  request here on <agent>: …". Hollow wrote it, so Retry and Edit never
 *  offer it (C3 review, `messageActions.ts` `hostWrittenTurn`): Retry would
 *  send it to the agent still picked, likely the one out of usage.
 *  `messageActions.test.ts` pins it to the Rust wording. */
const AGENT_CONTINUED_PROMPT = / ran out of usage before it could answer my last message.*, and it changed nothing\. Please continue that request here on .+: start from the task state and the conversation above, then carry on\./;
export function isAgentContinuedPrompt(text: string): boolean {
  return AGENT_CONTINUED_PROMPT.test(text);
}
