import { onCleanup, onMount } from 'solid-js';
import { registerToastRegion } from '../toastPlacement';
import '../styles/view-toasts.css';

/** A bounded, normal-flow shelf inside the full view's keyboard boundary. */
export function ViewToastRegion() {
  let region!: HTMLDivElement;
  onMount(() => { const unregister = registerToastRegion(region); onCleanup(unregister); });
  return <div ref={region} class="view-toast-region" />;
}
