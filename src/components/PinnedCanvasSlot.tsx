import { Show, Suspense, lazy } from 'solid-js';
import { hasCanvasPins } from '../store/canvasPinPresence';
const Pins = lazy(() => import('./PinnedCanvases').then(m => ({ default: m.PinnedCanvases })));
/** Keep the live renderer, parsers and data readers off the empty Home launch path. */
export function PinnedCanvasSlot(props: { workspaceId?: string }) {
  return <Show when={hasCanvasPins()}><Suspense><Pins workspaceId={props.workspaceId}/></Suspense></Show>;
}
