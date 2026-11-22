import { createSignal } from 'solid-js';

let regions: HTMLElement[] = [];
const [toastMount, setToastMount] = createSignal<HTMLElement>();
export { toastMount };

/** Full views share a stacking level; the last one in DOM order is on top.
 * Publish only a changed destination: opening an underlying view must not
 * remount Portal's container or release the active notice's held countdown. */
function refreshMount() {
  setToastMount(regions.filter(region => region.isConnected).reduce<HTMLElement | undefined>((top, region) =>
    !top || (top.compareDocumentPosition(region) & Node.DOCUMENT_POSITION_FOLLOWING) ? region : top, undefined));
}

export function registerToastRegion(region: HTMLElement): () => void {
  regions = [...regions, region];
  refreshMount();
  return () => { regions = regions.filter(item => item !== region); refreshMount(); };
}
