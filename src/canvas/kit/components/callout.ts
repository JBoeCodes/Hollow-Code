/** `<hc-callout tone="warning" title="Heads up">Body…</hc-callout>`: a note
 *  with a heading. The heading is the tone's icon and word plus the title;
 *  the body is the author's light DOM, untouched. The `title` attribute moves
 *  to `data-title` on upgrade so it doesn't also show as a hover tooltip. */
import { attr, KitElement, part, tone, toneMark } from '../registry';

export class HcCallout extends KitElement {
  static get observedAttributes(): string[] { return ['tone', 'title']; }

  protected render(): void {
    // Move the title first: removing it re-enters render via the attribute
    // callback, and this pass then rebuilds over whatever that one built.
    const moved = this.getAttribute('title');
    if (moved !== null) {
      this.setAttribute('data-title', moved);
      this.removeAttribute('title');
    }
    this.clearParts();
    const t = tone(this);
    const title = attr(this, 'data-title');
    if (!this.hasAttribute('role')) this.setAttribute('role', 'note');
    if (!title && t === 'neutral') return;
    const head = part('div', 'hc-callout-head');
    head.append(toneMark(t, !title));
    if (title) head.append(Object.assign(document.createElement('strong'), { className: 'hc-callout-title', textContent: title }));
    this.prepend(head);
  }
}
