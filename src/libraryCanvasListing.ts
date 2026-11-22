import type { CanvasSummary } from './generated/ipc';
import { galleryItems, mapLimit, type GalleryItem } from './canvasGallery';
import { latestRequest } from './latestRequest';

type Workspace = { id: string; title: string };
type Listing = { items: GalleryItem[]; failed: string[] };

export function canvasStoreListing(read: (id: string) => Promise<CanvasSummary[]>, deliver: (value: Listing) => void) {
  const requests = latestRequest();
  let pending: readonly Workspace[] | null = null;
  let running = false;
  let disposed = false;
  let active = 0;
  let scope = '';
  const lastGood = new Map<string, CanvasSummary[]>();
  const inFlight = new Map<string, Promise<CanvasSummary[]>>();
  const waiting = new Set<() => void>();
  const cancellations = new Set<() => void>();
  const pump = () => {
    while (!disposed && active < 6 && waiting.size) {
      const start = waiting.values().next().value!;
      waiting.delete(start); start();
    }
  };
  // Native IPC has no abort API. A timed-out read keeps its slot until the
  // actual promise settles; retries cannot pile six more reads on top of it.
  function boundedRead(id: string): Promise<CanvasSummary[]> {
    const existing = inFlight.get(id);
    if (existing) return existing;
    let start!: () => void;
    const promise = new Promise<CanvasSummary[]>((resolve, reject) => {
      let started = false;
      const cancel = () => {
        clearTimeout(timer); waiting.delete(start); cancellations.delete(cancel);
        // A queued read owns no native slot. A started, timed-out read stays
        // deduplicated until IPC actually settles, including across retries.
        if (!started) inFlight.delete(id);
        reject(new Error('Canvas listing timed out'));
      };
      const timer = setTimeout(cancel, 15_000);
      cancellations.add(cancel);
      start = () => {
        started = true; active++;
        const release = () => {
          active--; clearTimeout(timer); cancellations.delete(cancel);
          inFlight.delete(id); pump();
        };
        void (async () => read(id))().then(value => { release(); resolve(value); }, error => { release(); reject(error); });
      };
    });
    inFlight.set(id, promise); waiting.add(start); pump();
    return promise;
  }
  // Same-scope refreshes coalesce behind the running batch without starving
  // delivery. Only a changed workspace scope or disposal invalidates it.
  async function drain() {
    running = true;
    while (pending && !disposed) {
      const workspaces = pending;
      pending = null;
      const ticket = requests.peek();
      const failed: string[] = [];
      const stores = await mapLimit(workspaces, 6, async w => {
        let canvases: CanvasSummary[] = [];
        if (requests.isLatest(ticket)) {
          try {
            canvases = await boundedRead(w.id);
            if (requests.isLatest(ticket)) lastGood.set(w.id, canvases);
          } catch {
            failed.push(w.title);
            canvases = lastGood.get(w.id) ?? [];
          }
        }
        // Superseded placeholders never escape: the batch delivery is gated.
        return { workspaceId: w.id, workspaceTitle: w.title, canvases };
      });
      if (requests.isLatest(ticket)) deliver({ items: galleryItems(stores), failed });
    }
    running = false;
  }
  return {
    load(workspaces: readonly Workspace[]) {
      if (disposed) return;
      const nextScope = JSON.stringify(workspaces);
      if (scope !== nextScope) {
        scope = nextScope; requests.start();
        const current = new Set(workspaces.map(w => w.id));
        for (const id of lastGood.keys()) if (!current.has(id)) lastGood.delete(id);
      }
      pending = workspaces;
      if (!running) void drain();
    },
    dispose() {
      disposed = true; pending = null; requests.cancel();
      lastGood.clear();
      for (const cancel of cancellations) cancel();
    },
  };
}
