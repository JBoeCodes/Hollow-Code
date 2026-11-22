/** One tag, two contexts. A normal canvas is a static report, never a task
 * mutation. In hc-ask it composes the existing matrix field so validation,
 * keyboard radio behavior, exact JSON answers and answer locking stay shared.
 * The nested field is retained on rerender to preserve the person's choices. */
import { attr, h, KitElement, part, source } from '../registry';
import { checklistRows } from '../checklist';

export class HcChecklist extends KitElement {
  static get observedAttributes(): string[] { return ['label', 'name', 'required']; }
  private matrix?: HTMLElement;
  private upgrade = () => { if (this.isConnected && this.hasAttribute('data-hc-ready')) this.render(); };
  connectedCallback(): void { addEventListener('hc-ask-ready', this.upgrade); super.connectedCallback(); }
  disconnectedCallback(): void { removeEventListener('hc-ask-ready', this.upgrade); }

  protected render(): void {
    const original = source(this);
    const children = original.querySelectorAll('li');
    const els = children.length ? Array.from(children) : Array.from(original.children);
    const { rows, repeated } = checklistRows(els.length ? els.map(el => el.textContent || '') : (original.textContent || '').split('\n'));
    // Keep the field connected so a label update does not drop keyboard focus.
    for (const part of Array.from(this.children)) if (part.hasAttribute('data-hc-part') && part !== this.matrix) part.remove();
    const label = attr(this, 'label') || 'Checklist';
    const asking = !!this.closest('hc-ask');
    if (asking && customElements.get('hc-matrix')) {
      const matrix = this.matrix ??= part('hc-matrix', 'hc-checklist-input');
      matrix.setAttribute('data-hc-checklist', '');
      source(matrix).replaceChildren(...rows.map(row => h('div', '', row.label)));
      matrix.setAttribute('name', attr(this, 'name'));
      matrix.setAttribute('label', label);
      matrix.toggleAttribute('required', this.hasAttribute('required'));
      matrix.setAttribute('options', 'Pass | Fail | Skip');
      if (matrix.parentElement !== this) this.prepend(matrix);
    } else {
      this.matrix?.remove();
      const frame = part('section', 'hc-checklist-report', null, { 'aria-label': label });
      frame.append(h('div', 'hc-checklist-title', label));
      const list = h('ul', 'hc-checklist-list', null, { role: 'list' });
      for (const row of rows) {
        const li = h('li', 'hc-checklist-row', null, { 'data-done': String(row.done) });
        li.append(h('span', 'hc-checklist-mark', row.done ? '✓' : '○', { 'aria-hidden': 'true' }), h('span', '', row.label), h('span', 'hc-checklist-status', row.done ? 'Done' : 'Not done'));
        list.append(li);
      }
      frame.append(list);
      this.prepend(frame);
    }
    if (!rows.length) this.append(part('p', 'hc-checklist-note', 'No checks listed.'));
    if (repeated) this.append(part('p', 'hc-checklist-note', `${repeated} repeated ${repeated === 1 ? 'row shown' : 'rows shown'} once${asking ? '.' : '; first mark kept.'}`));
  }
}
