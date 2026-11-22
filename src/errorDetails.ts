/** What a recovery screen's "Copy details" copies: the error's message, its
 *  `cause` when that says more, and the stack. WKWebView's `Error.stack`
 *  holds frames only, so copying the stack alone lost what failed (10-23).
 *  Never throws: a cause that can't be stringified falls back to String(). */
export function errorDetails(error: unknown): string {
  const head = describe(error);
  const out = [head];
  const cause = error instanceof Error ? error.cause : undefined;
  if (cause !== undefined && cause !== null) {
    const said = describe(cause);
    if (said !== head && !head.endsWith(`: ${said}`)) out.push(`Cause: ${said}`);
  }
  const stack = error instanceof Error ? error.stack : undefined;
  if (stack) out.push(stack.startsWith(head) ? stack.slice(head.length).replace(/^\n/, '') : stack);
  return out.filter(Boolean).join('\n');
}

function describe(value: unknown): string {
  if (value instanceof Error) return `${value.name}: ${value.message}`;
  if (typeof value === 'string') return value;
  try { return JSON.stringify(value) ?? String(value); } catch { return String(value); }
}
