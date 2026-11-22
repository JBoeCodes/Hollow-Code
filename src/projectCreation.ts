/** UI validation only; the backend independently bounds titles and creates
 * managed folders/worktrees. No filesystem identity derives from this name. */
export function projectNameProblem(name: string, existing: readonly string[]): string {
  const value = name.trim();
  if (!value) return 'Give the project a name.';
  if ([...value].length > 100 || /[\x00-\x1f\x7f]/.test(value)) return 'Use a name of 1–100 characters on one line.';
  if (existing.some(title => title.trim().toLocaleLowerCase() === value.toLocaleLowerCase())) return 'A project with this name already exists. Open it from Projects or choose another name.';
  return '';
}
