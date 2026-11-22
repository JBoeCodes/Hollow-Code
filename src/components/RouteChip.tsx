// M3a follow-up (2026-11-15): the chip on a routed turn's receipt. A
// dispatcher COO's `sessions.route_to_runner` row says where the request
// went ("Sent to Mac mini’s COO") and opens that chat on that Mac, read live
// over the page as the machine strip opens it (`store/peerChat`). Nothing of
// that chat is kept here (S26). Lazy: ToolCall loads it only for a route row.
import { Show } from 'solid-js';
import { isRouteCall, routeChipLabel, routeReceipt } from '../routeReceipt';
import { openRemoteChat } from '../store/peerChat';
import { MachineGlyph } from './MachineBadge';
import '../styles/peer-machines.css';

/** The chip for a route call's `result`: nothing until that Mac took it
 *  (running, failed or nothing routed), or for any other call. */
export default function RouteChip(props: { name: string; input: unknown; result: string }) {
  const receipt = () => (isRouteCall(props.name, props.input) ? routeReceipt(props.result) : null);
  return (
    <Show when={receipt()}>{r => (
      <button type="button" class="toolcall-file openable route-chip" title={`Open that chat on ${r().machine}`}
        aria-label={`${routeChipLabel(r())}: open that chat`}
        onClick={e => {
          // A sibling of the row's toggle, above its stretched layer: the
          // click opens the chat, not the row.
          e.preventDefault();
          e.stopPropagation();
          void openRemoteChat(r().machineId, r().session);
        }}>
        <MachineGlyph size={12} />
        <span class="toolcall-file-name">{routeChipLabel(r())}</span>
      </button>
    )}</Show>
  );
}
