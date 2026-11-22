import { ErrorBoundary, lazy, Suspense } from 'solid-js';
import '../styles/doorway-landscape.css';
const ThresholdArt = lazy(() => import('./ThresholdArt'));

/** Static, local decoration. The transparent fade inherits every theme's
 * actual surface (including glass); content never sits over the artwork. */
export function DoorwayLandscape(props: { placement: 'home' | 'chat' }) {
  return <div class={`doorway-landscape doorway-landscape--${props.placement}`} aria-hidden="true">
    <ErrorBoundary fallback={null}><Suspense fallback={null}><ThresholdArt /></Suspense></ErrorBoundary>
  </div>;
}
