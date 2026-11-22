/** Browser-mock damaged state stores (B·F08). With localStorage
 *  `mockResultReviewsDamaged` the Results listings reject with the
 *  backend's message for a damaged store that was moved aside. The views must
 *  show that message (it is the only place the user learns where the copy
 *  went), not fall back to a blank list. */
const DAMAGED: Record<string, { flag: string; error: string }> = {
  workspace_results: { flag: 'mockResultReviewsDamaged', error: 'Results review state was damaged (expected value at line 1 column 1). It was moved to /mock/vault/workspaces/demo/.hollow/results.json.corrupt-20261002T120000.000Z; a fresh one starts now.' },
  list_all_results_peek: { flag: 'mockResultReviewsDamaged', error: 'Results review state could not be read (/mock/vault/workspaces/demo/.hollow/results.json): expected value at line 1 column 1. It was left as is; open Results to see it.' },
  list_all_results: { flag: 'mockResultReviewsDamaged', error: 'Results review state was damaged (expected value at line 1 column 1). It was moved to /mock/vault/workspaces/demo/.hollow/results.json.corrupt-20261002T120000.000Z; a fresh one starts now.' },
};

/** Throws the backend's damaged-store error for `command` when its mock flag is set. */
export function rejectIfDamagedStore(command: string): void {
  const d = DAMAGED[command];
  if (d && localStorage.getItem(d.flag)) throw d.error;
}
