import { ErrorBoundary, Show, Suspense, lazy } from 'solid-js';
import { emptyScene, type EmptyEvidence, type EmptyScene } from '../emptyScene';
const Drawing = lazy(() => import('./EmptyArtDrawing'));
/** Own Suspense/error boundary: decoration must never suspend or replace a working view. */
export function EmptyArt(props: { kind: EmptyScene; evidence: EmptyEvidence; compact?: boolean }) {
  const kind = () => emptyScene(props.kind, props.evidence);
  const space = () => <span class="empty-art-space" aria-hidden="true" style={{ display:'block', width:props.compact ? '80px' : '160px', height:props.compact ? '56px' : '112px', 'max-width':'100%', margin:'0 auto' }}/>;
  return <Show when={kind()}>{scene => <ErrorBoundary fallback={space()}><Suspense fallback={space()}><Drawing kind={scene()} compact={props.compact}/></Suspense></ErrorBoundary>}</Show>;
}
