/** `<hc-rank name="order" label="Build order">` with one item per line: an
 *  ordered list the person reorders by dragging an item, or by keyboard.
 *  Each item is a tab stop; Up and Down move the focused item one place,
 *  Home and End to the top or bottom, and a polite live region announces
 *  "<item>: position 2 of 6". Answer: every item, in the person's order. */
import { h, part, source, uid } from '../../registry';
import { AskField, rowsOf } from '../field';
import { move, parseOptions, rankTarget, type FieldRule } from '../model';

export class HcRank extends AskField {
  static get observedAttributes(): string[] { return ['label']; }
  readonly _kind = 'rank';
  _order: string[] = [];
  _list: HTMLElement | null = null;
  _live: HTMLElement | null = null;

  protected render(): void {
    const items = parseOptions(rowsOf(source(this))).map(o => o.label);
    // Keep the person's order while the items are the same set.
    const same = items.length === this._order.length && items.every(i => this._order.includes(i));
    if (!same) this._order = items;
    const frame = this._frame('group');
    const hintId = uid('hcq-hint');
    frame.append(h('p', 'hc-rank-hint', 'Drag an item, or focus it and press the Up and Down arrow keys to move it.', { id: hintId }));
    const list = h('ol', 'hc-rank-list');
    this._list = list;
    this._order.forEach(label => {
      const li = this._control(h('li', 'hc-rank-item', null, { tabindex: '0', 'aria-describedby': hintId }));
      li.append(h('span', 'hc-rank-pos', null, { 'aria-hidden': 'true' }), h('span', 'hc-opt-text', label), h('span', 'hc-rank-grip', '⠿', { 'aria-hidden': 'true' }));
      li.dataset.hcItem = label;
      li.addEventListener('keydown', e => this._key(e, li));
      li.addEventListener('pointerdown', e => this._drag(e, li));
      list.append(li);
    });
    this._live = part('p', 'hc-sr', null, { 'aria-live': 'polite' });
    frame.append(list);
    this.append(this._live);
    this._number();
    this._done(frame);
  }

  _items(): HTMLElement[] { return this._list ? (Array.from(this._list.children) as HTMLElement[]) : []; }

  /** Renumber after a move and read the order back from the list. */
  _number(): void {
    const items = this._items();
    items.forEach((li, i) => { li.firstElementChild!.textContent = `${i + 1}`; });
    this._order = items.map(li => li.dataset.hcItem || '');
  }

  _say(li: HTMLElement): void {
    const items = this._items();
    if (this._live) this._live.textContent = `${li.dataset.hcItem}: position ${items.indexOf(li) + 1} of ${items.length}`;
  }

  _key(e: KeyboardEvent, li: HTMLElement): void {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const items = this._items();
    const i = items.indexOf(li);
    const to = rankTarget(e.key, i, items.length);
    if (to === null) return;
    e.preventDefault();
    if (to !== i) this._place(li, to);
    li.focus();
    this._say(li);
  }

  /** Put `li` at index `to` in the list. */
  _place(li: HTMLElement, to: number): void {
    const list = this._list!;
    const order = move(this._items(), this._items().indexOf(li), to);
    list.insertBefore(li, order[to + 1] || null);
    this._number();
    this._changed();
  }

  /** Drag with the pointer. The move and end listeners sit on the window:
   *  moving the item in the DOM drops pointer capture, so the item itself
   *  would stop hearing the pointer after its first move. A pointer released
   *  outside the frame ends the drag at its next move (no buttons down). */
  _drag(e: PointerEvent, li: HTMLElement): void {
    if (e.button !== 0) return;
    e.preventDefault();
    li.focus();
    li.setAttribute('data-hc-dragging', '');
    const onMove = (m: PointerEvent) => {
      if (!m.buttons) return onUp();
      // The index whose place the pointer is over: how many other items'
      // middles are above it.
      const items = this._items();
      let to = 0;
      for (const other of items) {
        const r = other.getBoundingClientRect();
        if (other !== li && m.clientY > r.top + r.height / 2) to++;
      }
      if (to !== items.indexOf(li)) {
        this._place(li, to);
        li.focus();
      }
    };
    const onUp = () => {
      li.removeAttribute('data-hc-dragging');
      removeEventListener('pointermove', onMove);
      removeEventListener('pointerup', onUp);
      removeEventListener('pointercancel', onUp);
      this._say(li);
    };
    addEventListener('pointermove', onMove);
    addEventListener('pointerup', onUp);
    addEventListener('pointercancel', onUp);
  }

  _value(): string[] { return this._order.slice(); }
  _rule(): FieldRule { return { kind: 'rank', required: false }; }
}
