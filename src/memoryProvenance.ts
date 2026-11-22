/**
 * Where a memory came from, in words (next-level P18). A learning's
 * `sourceRunId` names the run it was proposed in; the review card shows it
 * next to the note it is saved to and the date it was approved, so a
 * memory is never an unexplained fact. Pure, unit-tested.
 */
export function memorySource(runId: string | null | undefined): string {
  const id = runId?.trim();
  if (!id) return 'No source run recorded (added by hand or before sources were kept).';
  if (/workflow|automation|heartbeat|routine/i.test(id)) return `An automation run (${id}).`;
  if (/chat|conversation|session|turn/i.test(id)) return `A conversation (${id}).`;
  return `Run ${id}.`;
}
