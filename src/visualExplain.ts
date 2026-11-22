/** "Explain visually" in a reply's ⋯ menu: the request it drafts into the
 *  composer. Whiteboards were removed on 2026-10-07; it asks for a canvas
 *  whose structure is an `hc-flow` diagram. The menu item is hidden when
 *  the chat's Canvases are off (the agent is told Markdown only there). */

export function visualExplanationPrompt(text: string): string {
  const excerpt = text.slice(0, 12_000);
  return `Explain the following response visually in a canvas (a hollow-canvas fence): an hc-flow diagram for the important relationships, steps and decisions, and other kit elements only where they help. For a code change, distinguish before/after or added/changed/removed behavior and include source references only where verified. Read relevant code or diffs if needed; label assumptions. Do not change files just to explain them.\n\nResponse to explain (quoted data${text.length > excerpt.length ? '; truncated to 12,000 characters' : ''}):\n${JSON.stringify(excerpt)}`;
}
