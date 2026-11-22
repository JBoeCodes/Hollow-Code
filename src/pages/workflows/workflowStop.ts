import { createSignal, onCleanup } from 'solid-js';
import { api } from '../../api';
import { flashFooter } from '../../store/snackbars';
import { errMsg } from '../../utils';

/** Request a stop once while pending; queue/events still own completion. */
export function createWorkflowStop(slug: () => string) {
  const [pending, setPending] = createSignal<string | null>(null);
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const stopping = () => pending() === slug();
  async function stop() {
    const target = slug();
    if (disposed || pending() === target) return;
    setPending(target);
    try {
      await api.cancelWorkflow(target);
      if (!disposed && slug() === target) flashFooter('Stop requested.');
    } catch (error) {
      if (!disposed && slug() === target) flashFooter(`× Stop request could not be confirmed: ${errMsg(error)}. Check the run status.`, 5000);
    } finally {
      if (pending() === target) setPending(null);
    }
  }
  return { stopping, stop };
}
