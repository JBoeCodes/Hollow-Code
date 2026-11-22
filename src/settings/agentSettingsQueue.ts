// One queue for every Settings piece that saves the agent settings file (the
// Models, Run limits and App builds forms all write the whole payload). A save
// reads the file, merges its own piece and writes it back; two of them in
// flight would both read before either writes, and the second write would
// revert the first piece's change. Chaining every read, merge and write on one
// module-level promise makes them run one after the other.
let tail: Promise<unknown> = Promise.resolve();

/** Run `job` after every job queued before it has settled (a failure does not block the next). */
export function serializeAgentSettingsWrite<T>(job: () => Promise<T>): Promise<T> {
  const run = tail.then(job, job);
  tail = run.catch(() => undefined);
  return run;
}
