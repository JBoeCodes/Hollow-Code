import { Icon } from '../icons';
import { openContextMenuAt, type ContextMenuItem } from '../store';
import { focusNextMenu } from '../store/listKeys';

/**
 * A visible "⋯" that opens an object's menu (consolidation D, 2026-10-13):
 * the same items its right-click opens, so no action is right-click only.
 * `label` names the object ("Actions for Q3 review"); `describedBy`, when a
 * row's ⋯ carries a short label, points at the row's own name.
 *
 * A native listener, not Solid's delegated `onClick`: delegated handlers run
 * at the document, where App's click-to-close handler would shut the menu
 * the moment it opened. Opened by Enter/Space (`detail === 0`), the menu
 * takes focus as the keyboard's `.` does.
 */
export function MoreButton(props: { label: string; items: () => ContextMenuItem[]; class?: string; disabled?: boolean; describedBy?: string }) {
  return <button type="button" class={`icon-btn more-btn${props.class ? ` ${props.class}` : ''}`} aria-label={props.label} title={props.label} aria-describedby={props.describedBy}
    aria-haspopup="menu" disabled={props.disabled}
    on:click={e => {
      e.preventDefault(); e.stopPropagation();
      if (e.detail === 0) focusNextMenu();
      openContextMenuAt(e.currentTarget as Element, props.items());
    }}>
    <Icon name="more" size={15} />
  </button>;
}
