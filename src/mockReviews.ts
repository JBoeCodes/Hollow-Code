/**
 * Browser-mock (`?mock=1`) versions of the page-review commands (Rust
 * `workspace_reviews.rs`, next-level P15), with the same rules: the review
 * JSON lives at `{workspace}/outputs/reviews/<slug>.annotations.json` in the
 * mock notes map (persisted in `mockWorkspaceNotes`, so Results lists it),
 * capture PNGs as data URLs in localStorage `mockReviewImages` keyed by their
 * path, never replacing a file, compare-and-swap on a content hash
 * (`REVIEW_CHANGED:`).
 */
import type { WorkWorkspace } from './workspaceTypes';

const COMMANDS = ['list_annotation_sets', 'read_annotation_set', 'save_annotation_set', 'add_annotation_capture'];
const SUFFIX = '.annotations.json';

function hash(text: string): string {
  let h = 0x811c9dc5 >>> 0;
  for (let i = 0; i < text.length; i++) { h ^= text.charCodeAt(i); h = Math.imul(h, 16777619) >>> 0; }
  return `${h.toString(16).padStart(8, '0')}${text.length.toString(16)}`;
}
const slug = (title: string) => title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 60).replace(/-+$/, '') || 'review';
const uuid = () => (globalThis.crypto?.randomUUID?.() ?? `mock-${Date.now().toString(16)}`);

type Capture = { url: string; title: string; capturedAt: string; viewport: { width: number; height: number }; imageSize: { width: number; height: number }; marks: unknown[]; pngBase64: string };

export function reviewsFixture(command: string, args: Record<string, unknown> | undefined, notes: Map<string, string>): { value: unknown } | null {
  if (!COMMANDS.includes(command)) return null;
  const persist = () => localStorage.setItem('mockWorkspaceNotes', JSON.stringify(Object.fromEntries(notes)));
  const images = (): Record<string, string> => JSON.parse(localStorage.getItem('mockReviewImages') ?? '{}');
  const id = String(args?.id ?? '');
  const workspace = (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as WorkWorkspace[]).find(w => w.id === id);
  if (!workspace) throw new Error('Workspace not found');
  const relOf = (path: string) => {
    const rel = (path.startsWith(`${workspace.path}/`) ? path.slice(workspace.path.length + 1) : path).replace(/^\/+/, '');
    if (!rel.startsWith('outputs/') || !rel.endsWith(SUFFIX) || rel.split('/').some(s => !s || s === '..' || s.startsWith('.'))) throw new Error(`${path} is not a review in workspace ${id}`);
    return rel;
  };
  const wire = (rel: string, text: string) => {
    const v = JSON.parse(text);
    const imgs = images();
    return {
      workspaceId: id, path: `${workspace.path}/${rel}`, file: rel, sha256: hash(text), id: v.id, title: v.title, createdAt: v.createdAt, updatedAt: v.updatedAt,
      captures: v.captures.map((c: Record<string, unknown>) => ({ ...c, imageDataUrl: imgs[`${workspace.path}/${c.image}`] ?? null })),
      links: v.links,
    };
  };
  const record = (c: Capture, cid: string, image: string, parent: string | null) => {
    const store = images();
    store[`${workspace.path}/${image}`] = `data:image/png;base64,${c.pngBase64.replace(/^data:image\/png;base64,/, '')}`;
    try { localStorage.setItem('mockReviewImages', JSON.stringify(store)); } catch { /* the review still saves */ }
    return { id: cid, url: c.url, title: c.title, capturedAt: c.capturedAt, viewport: c.viewport, imageSize: c.imageSize, image, marks: c.marks ?? [], parent };
  };
  const write = (rel: string, file: unknown) => { const text = JSON.stringify(file, null, 2) + '\n'; notes.set(`${workspace.path}/${rel}`, text); persist(); return text; };
  const log = (entry: unknown) => { const l = JSON.parse(localStorage.getItem('mockReviewLog') ?? '[]'); l.push(entry); localStorage.setItem('mockReviewLog', JSON.stringify(l.slice(-50))); };

  switch (command) {
    case 'list_annotation_sets': {
      const prefix = `${workspace.path}/outputs/`;
      const out = [...notes].filter(([p]) => p.startsWith(prefix) && p.endsWith(SUFFIX)).map(([p, text]) => {
        const v = JSON.parse(text);
        const first = v.captures[0];
        const requested = first?.marks?.length ?? 0;
        let done = 0;
        for (let i = 0; i < requested; i++) if ([...v.links].reverse().find((l: { capture: string; mark: number }) => l.capture === first.id && l.mark === i)?.status === 'done') done++;
        return { path: p, file: p.slice(workspace.path.length + 1), id: v.id, title: v.title, updatedAt: v.updatedAt, url: first?.url ?? null, captures: v.captures.length, requested, done, error: null };
      });
      return { value: out.sort((a, b) => String(b.updatedAt).localeCompare(String(a.updatedAt))) };
    }
    case 'read_annotation_set': {
      const rel = relOf(String(args?.path));
      const text = notes.get(`${workspace.path}/${rel}`);
      if (text == null) throw new Error('This review no longer exists.');
      return { value: wire(rel, text) };
    }
    case 'save_annotation_set': {
      const input = args?.input as { title?: string; capture: Capture };
      if (!/^https?:\/\//i.test(input.capture.url)) throw new Error('REVIEW_INVALID: a capture\'s url must be an http(s) address');
      const now = new Date().toISOString();
      const title = (input.title?.trim() || (input.capture.title ? `Review: ${input.capture.title}` : 'Page review')).slice(0, 180);
      const stem = slug(title);
      let n = 1, rel = '';
      do { rel = `outputs/reviews/${n === 1 ? stem : `${stem}-${n}`}${SUFFIX}`; n++; } while (notes.has(`${workspace.path}/${rel}`));
      const image = `${rel.slice(0, -SUFFIX.length)}/c1.png`;
      const file = { hollowAnnotations: 1, id: uuid(), title, workspaceId: id, createdAt: now, updatedAt: now, captures: [record(input.capture, 'c1', image, null)], links: [] };
      log({ command, rel, marks: input.capture.marks.length, url: input.capture.url, viewport: input.capture.viewport, capturedAt: input.capture.capturedAt });
      return { value: wire(rel, write(rel, file)) };
    }
    case 'add_annotation_capture': {
      const rel = relOf(String(args?.path));
      const text = notes.get(`${workspace.path}/${rel}`);
      if (text == null) throw new Error('This review no longer exists.');
      if (hash(text) !== args?.expectedSha256) throw new Error('REVIEW_CHANGED: this review was changed elsewhere since you opened it; reopen it to see the newest version');
      const v = JSON.parse(text);
      const now = new Date().toISOString();
      let k = 1; while (v.captures.some((c: { id: string }) => c.id === `c${k}`)) k++;
      const cid = `c${k}`;
      const parent = v.captures.at(-1)?.id ?? null;
      v.captures.push(record(args?.capture as Capture, cid, `${rel.slice(0, -SUFFIX.length)}/${cid}.png`, parent));
      for (const l of (args?.links ?? []) as { capture: string; mark: number; status: string; note?: string }[]) v.links.push({ capture: l.capture, mark: l.mark, result: cid, status: l.status, note: l.note ?? '', at: now });
      v.updatedAt = now;
      log({ command, rel, links: (args?.links as unknown[] | undefined)?.length ?? 0 });
      return { value: wire(rel, write(rel, v)) };
    }
  }
  return null;
}
