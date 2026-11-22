/** The kit's element base and registry (Canvas C2).
 *
 *  Rules every component follows, so later PRs slot in without revisiting
 *  them:
 *  - **Light DOM only.** The kit stylesheet styles the document, so the host
 *    theme (`--app-*`, kept live by the `theme` bridge message) reaches every
 *    part. No shadow roots.
 *  - **Readable before upgrade.** A component never hides its own children,
 *    and `kit.css` gives each attribute-only component a `:not([data-hc-ready])`
 *    text fallback, so a document stays legible when the kit script doesn't
 *    run (before upgrade, or wherever the kit isn't inlined).
 *  - **Text, never markup.** Attribute values reach the page through
 *    `textContent` only. The kit has no network, no `eval`, and posts no
 *    bridge message itself: `hc-file` renders an `a[data-vault]`, which the
 *    host runtime already turns into a `navigate` request, and view state
 *    goes through the runtime's `hollowCanvas` (below).
 *  - **View state persists (Canvas C3).** A component that holds view state
 *    the person chose (`hc-tabs`' tab, `hc-table`'s sort, `hc-details`' open)
 *    keeps it with `keepView` when the person changes it, and implements
 *    `_restore()`, which runs after its first render once the host's init
 *    has arrived, and again on `hollow-init`. Keys are `hc:<tag>#<id>` or
 *    `hc:<tag>:<index among that tag>`; values name what was chosen (a tab's
 *    label, a column's header), so a revision whose inputs are gone falls
 *    back to its default. Restoring never saves.
 *  - **Data stays in the light DOM.** A data component's source (text rows,
 *    a table, code) is the author's own children. On upgrade they move into
 *    one hidden `[data-hc-source]` wrapper (`source()`), so the document reads
 *    the same without the script and the rendered parts replace it visually.
 *  - **No ResizeObserver.** It goes dead in sandboxed WKWebView frames (see
 *    `sandbox-doc.js`). Components that measure listen for the window's
 *    `resize` and the kit's own `hc-reflow` event (`reflow()`), which a
 *    component fires when it reveals content (tabs, details).
 *  - **Rendered parts are marked** `data-hc-part`, so a re-render (an
 *    attribute change, a move in the DOM) replaces them and leaves the
 *    author's own children alone. */

import { KIT_TONES, type KitTone } from './manifest';
import { schemeOf } from './palette';
import { parseRows, type Rows } from './data';

/** Run `fn` once the parser has finished. The kit loads in `<head>`, so an
 *  element parsed in `<body>` connects before its children exist. */
export function whenParsed(fn: () => void): void {
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', fn, { once: true });
  else fn();
}

/** An attribute's trimmed value ('' when absent). */
export function attr(el: Element, name: string): string {
  return (el.getAttribute(name) || '').trim();
}

/** Give `el` a role and, when `label` is set, an accessible name. */
export function named(el: Element, role: string, label: string): void {
  el.setAttribute('role', role);
  if (label) el.setAttribute('aria-label', label);
  else el.removeAttribute('aria-label');
}

export function tone(el: Element): KitTone {
  const t = attr(el, 'tone').toLowerCase();
  return Object.prototype.hasOwnProperty.call(KIT_TONES, t) ? (t as KitTone) : 'neutral';
}

/** Create an element with a class, optional text and attributes. */
export function h(tag: string, cls: string, text?: string | null, attrs?: Record<string, string>): HTMLElement {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text != null) el.textContent = text;
  if (attrs) for (const k of Object.keys(attrs)) el.setAttribute(k, attrs[k]);
  return el;
}

/** A rendered part: removed and rebuilt on every render. */
export function part(tag: string, cls: string, text?: string | null, attrs?: Record<string, string>): HTMLElement {
  const el = h(tag, cls, text, attrs);
  el.setAttribute('data-hc-part', '');
  return el;
}

/** The tone's icon (hidden from assistive tech) plus its word, visible or
 *  for screen readers only. */
export function toneMark(t: KitTone, visibleWord: boolean): DocumentFragment {
  const f = document.createDocumentFragment();
  f.append(h('span', `hc-icon hc-icon-${t}`, KIT_TONES[t].icon, { 'aria-hidden': 'true' }));
  f.append(h('span', visibleWord ? 'hc-tone-word' : 'hc-sr', visibleWord ? KIT_TONES[t].word : `${KIT_TONES[t].word}: `));
  return f;
}

let ids = 0;
export function uid(prefix: string): string {
  ids += 1;
  return `${prefix}-${ids}`;
}

/** The host runtime's state API (`sandbox-doc.js` job 5); absent where the
 *  runtime didn't run. */
interface HollowCanvas { init: object | null; get(key: string): unknown; set(key: string, value: unknown): boolean }
const hollow = (): HollowCanvas | undefined => (window as unknown as { hollowCanvas?: HollowCanvas }).hollowCanvas;

function viewKey(el: Element): string {
  const tag = el.localName;
  return `hc:${tag}${el.id ? `#${el.id}` : `:${Array.prototype.indexOf.call(document.getElementsByTagName(tag), el)}`}`;
}

/** What the person last chose in `el` (from the saved state), if anything. */
export function savedView(el: Element): unknown {
  return hollow()?.get(viewKey(el));
}

/** Save what the person just chose in `el`. Call only from their own input. */
export function keepView(el: Element, value: unknown): void {
  hollow()?.set(viewKey(el), value);
}

/** Base for every kit element. Subclasses implement `render()`, which must be
 *  idempotent: it is called on first connect (after parsing) and again when an
 *  observed attribute changes. */
export abstract class KitElement extends HTMLElement {
  protected abstract render(): void;

  /** Apply the saved view state (components that keep one override this). */
  _restore(): void {}

  connectedCallback(): void {
    whenParsed(() => {
      if (!this.isConnected) return;
      this.render();
      this.setAttribute('data-hc-ready', '');
      if (hollow()?.init) this._restore();
    });
  }

  attributeChangedCallback(): void {
    if (this.hasAttribute('data-hc-ready')) this.render();
  }

  protected clearParts(): void {
    for (const el of Array.from(this.children)) if (el.hasAttribute('data-hc-part')) el.remove();
  }
}

/** Define `name` unless something already did (a document may inline the kit
 *  twice; the second copy must not throw). */
export function define(name: string, ctor: CustomElementConstructor): void {
  if (!customElements.get(name)) customElements.define(name, ctor);
}

/** The author's source children, gathered into one hidden wrapper (created on
 *  first use; nodes added later are swept in too). Never a rendered part, so
 *  re-renders keep it. */
export function source(el: HTMLElement): HTMLElement {
  let box = Array.from(el.children).find(c => c.hasAttribute('data-hc-source')) as HTMLElement | undefined;
  if (!box) {
    box = h('div', '', null, { 'data-hc-source': '' });
    box.hidden = true;
  }
  const stray = Array.from(el.childNodes).filter(n => n !== box && !(n instanceof Element && n.hasAttribute('data-hc-part')));
  box.append(...stray);
  if (!box.isConnected) el.append(box);
  return box;
}

/** Rows from a source wrapper: its `<table>` (the first row is a header when
 *  every cell in it is a `<th>`), or its text, one row per line. */
export function sourceRows(box: HTMLElement): { rows: Rows; header?: boolean } {
  const table = box.querySelector('table');
  if (!table) return { rows: parseRows(box.textContent || '') };
  const rows = Array.from(table.rows);
  return {
    rows: rows.map(r => Array.from(r.cells).map(c => (c.textContent || '').trim())),
    header: rows.length > 0 && Array.from(rows[0].cells).every(c => c.tagName === 'TH'),
  };
}

/** Tell measuring components (charts) that layout may have changed. */
export function reflow(): void {
  dispatchEvent(new Event('hc-reflow'));
}

/** Set `data-hc-scheme` on the root from the live `--app-surface` token, so
 *  the chart palette's light or dark steps follow the host theme. A canvas
 *  normalises any CSS colour to hex or rgba for `schemeOf`. */
let probe: CanvasRenderingContext2D | null | undefined;
export function syncScheme(): void {
  const root = document.documentElement;
  const value = getComputedStyle(root).getPropertyValue('--app-surface').trim();
  let colour = value;
  if (probe === undefined) probe = document.createElement('canvas').getContext('2d');
  if (probe && value) {
    probe.fillStyle = '#fff';
    probe.fillStyle = value;
    colour = String(probe.fillStyle);
  }
  root.setAttribute('data-hc-scheme', colour ? schemeOf(colour) : 'light');
}
