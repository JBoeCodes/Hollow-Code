/** `<hc-stats label="Release">` holding `<hc-stat label="Bed checks"
 *  value="29" delta="0 fail" tone="success">` tiles. A stat renders its label,
 *  value and delta as text; a toned delta (or, with no delta, the label)
 *  also carries the tone's icon and a screen-reader word, and an untoned delta gets an arrow from its sign. Any
 *  children the author wrote stay below as a note. */
import { attr, h, KitElement, named, part, tone, toneMark } from '../registry';

export class HcStats extends KitElement {
  static get observedAttributes(): string[] { return ['label']; }

  protected render(): void {
    const label = attr(this, 'label');
    named(this, 'group', label);
  }
}

export class HcStat extends KitElement {
  static get observedAttributes(): string[] { return ['label', 'value', 'delta', 'tone']; }

  protected render(): void {
    this.clearParts();
    const t = tone(this);
    const label = attr(this, 'label');
    const value = attr(this, 'value');
    const delta = attr(this, 'delta');
    named(this, 'group', label);

    const parts: HTMLElement[] = [];
    // 10-11 B2: a tone with no delta still shows (icon and word by the
    // label; the tile's top edge takes the colour), and a long value steps
    // down a size instead of wrapping into a tower.
    if (label) {
      const head = part('div', 'hc-stat-label', label);
      if (!delta && t !== 'neutral') head.prepend(toneMark(t, false));
      parts.push(head);
    }
    const v = part('div', 'hc-stat-value', value || '—');
    if (!label && !delta && t !== 'neutral') v.prepend(toneMark(t, false));
    if (value.length > 10) v.dataset.long = '';
    parts.push(v);
    if (delta) {
      const row = part('div', 'hc-stat-delta');
      if (t !== 'neutral') row.append(toneMark(t, false));
      else if (/^[+↑▲]/.test(delta)) row.append(h('span', 'hc-icon', '▲', { 'aria-hidden': 'true' }));
      else if (/^[-−↓▼]/.test(delta)) row.append(h('span', 'hc-icon', '▼', { 'aria-hidden': 'true' }));
      row.append(h('span', '', delta));
      parts.push(row);
    }
    this.prepend(...parts);
  }
}
