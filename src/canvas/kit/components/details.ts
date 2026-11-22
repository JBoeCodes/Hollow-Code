/** `<hc-details summary="Why" open>Body…</hc-details>`: a disclosure. On
 *  upgrade the author's children move into a body region behind a real
 *  `<button aria-expanded>`, so it is keyboard-operable (Enter, Space) with
 *  a visible focus ring. Children added later are swept into the body too.
 *  Before upgrade the summary shows as a label and the body stays visible.
 *  Opening or closing it persists in the canvas state, restored on init. */
import { attr, h, keepView, KitElement, part, reflow, savedView, uid } from '../registry';

export class HcDetails extends KitElement {
  static get observedAttributes(): string[] { return ['summary', 'open']; }
  private body: HTMLElement | null = null;
  private button: HTMLButtonElement | null = null;
  private watcher: MutationObserver | null = null;

  protected render(): void {
    if (!this.body || !this.button) {
      const id = uid('hc-details');
      this.button = part('button', 'hc-details-toggle', null, { type: 'button', 'aria-controls': id }) as HTMLButtonElement;
      this.button.addEventListener('click', () => keepView(this, this.toggleAttribute('open')));
      this.body = part('div', 'hc-details-body', null, { id, role: 'region' });
      this.adopt();
      this.prepend(this.button, this.body);
      this.watcher = new MutationObserver(() => this.adopt());
      this.watcher.observe(this, { childList: true });
    }
    const open = this.hasAttribute('open');
    const summary = attr(this, 'summary') || 'Details';
    this.button.replaceChildren(h('span', 'hc-details-chevron', '›', { 'aria-hidden': 'true' }), h('span', 'hc-details-summary', summary));
    this.button.setAttribute('aria-expanded', String(open));
    this.body.setAttribute('aria-label', summary);
    this.body.hidden = !open;
    if (open) reflow();
  }

  _restore(): void {
    const open = savedView(this);
    if (typeof open === 'boolean') this.toggleAttribute('open', open);
  }

  /** Move every child that isn't one of our parts into the body. */
  private adopt(): void {
    const body = this.body;
    if (!body) return;
    const stray = Array.from(this.childNodes).filter(n => n !== body && n !== this.button);
    if (stray.length) body.append(...stray);
  }

  disconnectedCallback(): void {
    // Keep the parts (a moved element re-connects); only stop the observer
    // when it is really gone, which a later connect re-creates.
    this.watcher?.disconnect();
    this.watcher = null;
  }

  connectedCallback(): void {
    if (this.body && !this.watcher) {
      this.watcher = new MutationObserver(() => this.adopt());
      this.watcher.observe(this, { childList: true });
    }
    super.connectedCallback();
  }
}
