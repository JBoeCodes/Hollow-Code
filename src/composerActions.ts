/** Navigate only after an accepted send, while the originating view still owns
 * the screen. The destination rechecks the same guard during its async open. */
export async function sendThenBackground(send: () => Promise<boolean>, stillHere: () => boolean, open: (guard: () => boolean) => Promise<boolean>) {
  if (!await send() || !stillHere()) return false;
  return open(stillHere);
}
