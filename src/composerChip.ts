/** Where the composer sends a message. */
export type ComposerDestination =
  | { kind: 'project'; title: string }
  | { kind: 'quick' }
  | { kind: 'main' }
  | { kind: 'vault' };

/** The message box's resting placeholder: who answers and, in a project,
 *  which one. Chats without a project name only the agent. */
export function composerPlaceholder(agent: string, where: ComposerDestination): string {
  if (where.kind === 'main') return 'Message the COO…';
  if (where.kind === 'project' && where.title.trim()) return `Message ${agent} in ${where.title.trim()}…`;
  return `Message ${agent}…`;
}
