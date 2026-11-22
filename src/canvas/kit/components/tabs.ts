/** `<hc-tabs label="Views">` with one child per panel:
 *  `<section label="Chart">…</section>`; a child with `selected` starts
 *  selected. ARIA tabs with a roving tabindex: Left/Right (wrapping),
 *  Home and End move focus and select; Tab moves into the panel. Without
 *  the script every panel shows under its label. Selecting a panel fires
 *  `hc-reflow`, so a chart inside it draws at its real width. The chosen tab
 *  (its label) persists in the canvas state and is restored on init. */
import { attr, keepView, KitElement, part, reflow, savedView, uid } from '../registry';

const tabLabel = (panel: Element, i: number): string => attr(panel, 'label') || `Tab ${i + 1}`;

export class HcTabs extends KitElement {
  static get observedAttributes(): string[] { return ['label']; }
  private _selected = -1;

  private _panels(): HTMLElement[] {
    return Array.from(this.children).filter(c => !c.hasAttribute('data-hc-part')) as HTMLElement[];
  }

  protected render(): void {
    this.clearParts();
    const panels = this._panels();
    if (this._selected < 0 || this._selected >= panels.length) {
      this._selected = Math.max(0, panels.findIndex(p => p.hasAttribute('selected')));
    }
    const label = attr(this, 'label');
    const list = part('div', 'hc-tablist', null, { role: 'tablist' });
    if (label) list.setAttribute('aria-label', label);
    panels.forEach((panel, i) => {
      const tabId = uid('hc-tab');
      if (!panel.id) panel.id = uid('hc-panel');
      const tab = document.createElement('button');
      tab.type = 'button';
      tab.id = tabId;
      tab.className = 'hc-tab';
      tab.textContent = tabLabel(panel, i);
      tab.setAttribute('role', 'tab');
      tab.setAttribute('aria-controls', panel.id);
      tab.addEventListener('click', () => this._select(i, false, true));
      list.append(tab);
      panel.setAttribute('role', 'tabpanel');
      panel.setAttribute('aria-labelledby', tabId);
      panel.tabIndex = 0;
    });
    list.addEventListener('keydown', e => {
      const n = panels.length;
      const k = e.key;
      const next = k === 'ArrowRight' ? (this._selected + 1) % n
        : k === 'ArrowLeft' ? (this._selected - 1 + n) % n
          : k === 'Home' ? 0 : k === 'End' ? n - 1 : -1;
      if (next < 0 || !n) return;
      e.preventDefault();
      this._select(next, true, true);
    });
    this.prepend(list);
    this._select(this._selected, false);
  }

  _restore(): void {
    const chosen = savedView(this);
    const i = this._panels().findIndex((p, n) => tabLabel(p, n) === chosen);
    if (i >= 0) this._select(i, false);
  }

  private _select(index: number, focus: boolean, keep?: boolean): void {
    this._selected = index;
    const panel = this._panels()[index];
    if (keep && panel) keepView(this, tabLabel(panel, index));
    const tabs = Array.from(this.querySelectorAll<HTMLElement>(':scope > .hc-tablist > [role=tab]'));
    this._panels().forEach((panel, i) => {
      const on = i === index;
      panel.hidden = !on;
      const tab = tabs[i];
      if (!tab) return;
      tab.setAttribute('aria-selected', String(on));
      tab.tabIndex = on ? 0 : -1;
      if (on && focus) tab.focus();
    });
    reflow();
  }
}
