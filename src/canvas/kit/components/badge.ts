/** `<hc-badge tone="success">Merged</hc-badge>`: a short status label. The
 *  text is the author's own light DOM; the tone adds an icon (from `kit.css`,
 *  so it shows before upgrade too) and a colour. */
import { KitElement } from '../registry';

export class HcBadge extends KitElement {
  protected render(): void {
    /* Styling only: the icon is CSS-generated, the text is the author's. */
  }
}
