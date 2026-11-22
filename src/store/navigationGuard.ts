// A mounted editor may defer navigation before any destination state changes.
// Confirmation UI stays in the lazy editor; clean navigation stays synchronous.
type Guard = () => boolean | Promise<boolean>;
// Guards compose (Settings PR 4 review): the automation editor's and the
// windows fold's can be mounted together, each with its own dispose. A
// navigation is held until every guard allows it; they are asked in
// registration order, one at a time, and the first refusal stops it.
const guards: Guard[] = [];
let request = 0;
let applying = 0;

export function registerNavigationGuard(guard: Guard): () => void {
  guards.push(guard);
  request++;
  return () => {
    const at = guards.indexOf(guard);
    if (at >= 0) { guards.splice(at, 1); request++; }
  };
}

/** A delayed answer belongs to this set of editors and this destination only.
 * `immediate` preserves existing motion for clean synchronous navigation. */
export function guardNavigation<T>(action: () => T, immediate: () => T = action): T | Promise<T | undefined> | undefined {
  if (applying) return action();
  const owners = [...guards];
  const token = ++request;
  const ask = (from: number): boolean | Promise<boolean> => {
    for (let i = from; i < owners.length; i++) {
      const answer = owners[i]();
      if (typeof answer !== 'boolean') return answer.then(ok => ok && ask(i + 1));
      if (!answer) return false;
    }
    return true;
  };
  const allowed = ask(0);
  const apply = (ok: boolean, go: () => T) => {
    if (!ok || token !== request) return;
    applying++;
    try { return go(); } finally { applying--; }
  };
  return typeof allowed === 'boolean' ? apply(allowed, immediate)
    : allowed.then(ok => apply(ok, action), () => undefined);
}
