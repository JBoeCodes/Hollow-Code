/** `<hc-compare label="Hosting">` with one child per option:
 *  `<div label="Fly" recommended>…</div>`. Options sit side by side (and
 *  stack when narrow), each a named group with its label as a heading; a
 *  `recommended` option carries a star and the word, never colour alone. */
import { attr, h, KitElement, named, part } from '../registry';

export class HcCompare extends KitElement {
  static get observedAttributes(): string[] { return ['label']; }

  protected render(): void {
    const label = attr(this, 'label');
    named(this, 'group', label);
    for (const option of Array.from(this.children)) {
      for (const p of Array.from(option.children)) if (p.hasAttribute('data-hc-part')) p.remove();
      const name = attr(option, 'label');
      named(option, 'group', name);
      const head = part('div', 'hc-compare-head');
      if (name) head.append(h('strong', 'hc-compare-label', name));
      if (option.hasAttribute('recommended')) {
        const badge = h('span', 'hc-compare-pick');
        badge.append(h('span', 'hc-icon', '★', { 'aria-hidden': 'true' }), h('span', '', 'Recommended'));
        head.append(badge);
      }
      if (head.childNodes.length) option.prepend(head);
    }
  }
}
