// Refuse a CARGO_TARGET_DIR cargo would put somewhere unintended: a relative
// path (resolved against whatever folder cargo runs in) or one with a segment
// starting with `~` (a tilde the shell never expanded, e.g. a quoted
// `"~/Library/Caches/x"`). On 2026-09-27 agent commands left 17.9 GB of cargo
// output in literal `src-tauri/~/Library/Caches/…` folders under five worktrees.
// The same rule runs in src-tauri/build.rs (src-tauri/src/cargo_target_rule.rs)
// for cargo commands that skip npm.
//
//   node scripts/check-cargo-target.mjs     exit 1 with the reason if $CARGO_TARGET_DIR is unsafe
import { realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

/** Why `value` is unsafe as CARGO_TARGET_DIR, or null (unset counts as fine). */
export function cargoTargetProblem(value) {
  if (value === undefined || value === '') return null;
  if (value.split('/').some(segment => segment.startsWith('~'))) {
    return `CARGO_TARGET_DIR is "${value}": a "~" there is a literal folder name (the shell didn't expand it), so cargo would write into a folder called "~" under the current one. Use an absolute path, e.g. "$HOME/Library/Caches/…".`;
  }
  if (!value.startsWith('/')) {
    return `CARGO_TARGET_DIR is "${value}": a relative path lands wherever cargo runs. Use an absolute path, e.g. "$HOME/Library/Caches/…".`;
  }
  return null;
}

// Compare real paths: a symlinked argv[1] must not skip the check silently.
const isMain = () => {
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
};

if (process.argv[1] && isMain()) {
  const problem = cargoTargetProblem(process.env.CARGO_TARGET_DIR);
  if (problem) {
    console.error(`check-cargo-target: ${problem}`);
    process.exit(1);
  }
}
