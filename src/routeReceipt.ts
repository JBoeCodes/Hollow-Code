// M3a follow-up (2026-11-15): a dispatcher COO's `sessions.route_to_runner`
// call in a transcript. Its receipt (the action's answer, Rust
// `dispatcher_coo::route`) names the Mac and the chat the request went to,
// never that chat's words (S26), so the row's chip can open that chat there
// (`store/peerChat.openRemoteChat`). Pure, unit-tested; the chip is the lazy
// `components/RouteChip.tsx`.

/** The routing action's id (Rust `dispatcher_coo::ACTION`). */
export const ROUTE_ACTION = 'sessions.route_to_runner';

/** Where a routed request went, from its receipt. */
export interface RouteReceipt {
  machineId: string;
  /** The Mac's name as the dispatcher says it. */
  machine: string;
  /** The chat there that took it (the runner COO's, or a lead's). */
  session: string;
  /** `coo` or `lead`. */
  to: 'coo' | 'lead';
}

/** Whether a tool call is Hollow's `run_action` with the routing action
 *  (Claude names it `mcp__hollow_app__run_action`). */
export function isRouteCall(name: string, input: unknown): boolean {
  const parts = name.split('__');
  if (parts[0] !== 'mcp' || parts[1] !== 'hollow_app' || parts[parts.length - 1] !== 'run_action') return false;
  return !!input && typeof input === 'object' && (input as Record<string, unknown>).id === ROUTE_ACTION;
}

/** The receipt in a route call's result, or null while it runs, when it
 *  failed (an error is a sentence, not JSON) or when it routed nothing.
 *  The broker may add a note as a second block, so only the first block
 *  is read when the whole result isn't JSON. */
export function routeReceipt(result: string | null | undefined): RouteReceipt | null {
  if (!result) return null;
  const parsed = parse(result) ?? parse(result.split('\n\n')[0] ?? '');
  if (!parsed || parsed.routed !== true) return null;
  const machine = parsed.machine && typeof parsed.machine === 'object' ? (parsed.machine as Record<string, unknown>) : {};
  const machineId = typeof machine.id === 'string' ? machine.id : '';
  const session = typeof parsed.session === 'string' ? parsed.session : '';
  if (!machineId || !session) return null;
  return {
    machineId,
    machine: typeof machine.name === 'string' && machine.name.trim() ? machine.name.trim() : 'the runner',
    session,
    to: parsed.to === 'lead' ? 'lead' : 'coo',
  };
}

/** The chip's words: where it went. */
export function routeChipLabel(r: RouteReceipt): string {
  return r.to === 'lead' ? `Sent to a lead on ${r.machine}` : `Sent to ${r.machine}’s COO`;
}

function parse(text: string): Record<string, unknown> | null {
  try {
    const v: unknown = JSON.parse(text);
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}
