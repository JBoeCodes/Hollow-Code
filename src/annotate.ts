/** Annotate in the side panel's browser (BrowserAnnotate.tsx): the marks a
 *  user draws on a snapshot of the page, and the text that goes to the chat
 *  with the marked-up image. Pure, so it's unit-tested. Mark coordinates are
 *  in the snapshot's own pixels. */

export type Mark =
  | { kind: 'page'; note: string }
  | { kind: 'pin'; x: number; y: number; note: string }
  | { kind: 'box'; x: number; y: number; w: number; h: number; note: string }
  | { kind: 'draw'; points: [number, number][]; note: string };

export type SpatialMark = Exclude<Mark, { kind: 'page' }>;

/** What browser_inspect reports under a mark (every field capped in Rust). */
export interface PageElement { tag: string; id: string; classes: string; text: string; label: string; href: string; selector: string }
export interface PageInspection { url: string; title: string; elements: (PageElement | null)[] }

/** The point a mark is about: a pin's tip, a box's centre, a drawing's start. */
export function markAnchor(m: SpatialMark): { x: number; y: number } {
  if (m.kind === 'pin') return { x: m.x, y: m.y };
  if (m.kind === 'box') return { x: m.x + m.w / 2, y: m.y + m.h / 2 };
  return { x: m.points[0][0], y: m.points[0][1] };
}

/** A box from any two corners, never negative. */
export function normalizeBox(x0: number, y0: number, x1: number, y1: number) {
  return { x: Math.min(x0, x1), y: Math.min(y0, y1), w: Math.abs(x1 - x0), h: Math.abs(y1 - y0) };
}

/** Snapshot pixels → the page's viewport CSS pixels (the snapshot is taken
 *  at the screen's scale, so usually 2×). */
export function toViewport(p: { x: number; y: number }, image: { width: number; height: number }, view: { width: number; height: number }): [number, number] {
  return [
    Math.round((p.x * view.width) / Math.max(1, image.width)),
    Math.round((p.y * view.height) / Math.max(1, image.height)),
  ];
}

/** `<button#save.btn.primary> "Save"`, short enough to scan. */
export function describeElement(el: PageElement | null | undefined): string {
  if (!el || !el.tag) return '';
  const classes = el.classes.split(/\s+/).filter(Boolean).slice(0, 2).map(c => `.${c}`).join('');
  const name = `<${el.tag}${el.id ? `#${el.id}` : ''}${classes}>`;
  const words = el.label || el.text;
  const quoted = words ? ` "${words.length > 80 ? `${words.slice(0, 80)}…` : words}"` : '';
  const link = el.href ? ` → ${el.href}` : '';
  const path = el.selector && el.selector !== el.tag ? ` (\`${el.selector}\`)` : '';
  return `${name}${quoted}${link}${path}`;
}

const KIND_LABEL: Record<Mark['kind'], string> = { pin: '', box: 'Box: ', draw: 'Drawing: ', page: 'Whole page: ' };

/** The draft text sent with the image: which page, then one numbered line per
 *  mark with the user's note and the page element under it. */
export function annotationText(page: { url: string; title: string }, marks: Mark[], inspection: PageInspection | null, capture?: string): string {
  const title = (inspection?.title || page.title || '').trim();
  const url = inspection?.url || page.url;
  // `capture` (next-level P15) says when and at what size it was captured,
  // so the agent reads it as a snapshot, not the live page.
  const head = `I marked up ${title ? `“${title}” (${url})` : url}; the annotated screenshot is attached.${capture ? ` ${capture}` : ''}`;
  if (!marks.length) return head;
  const lines = marks.map((m, i) => {
    const note = m.note.trim() || '(no comment)';
    const el = m.kind === 'draw' || m.kind === 'page' ? '' : describeElement(inspection?.elements[i]);
    const where = el ? ` — ${m.kind === 'box' ? 'around' : 'on'} ${el}` : '';
    return `${i + 1}. ${KIND_LABEL[m.kind]}${note}${where}`;
  });
  return `${head}\n\n${lines.join('\n')}`;
}
