/** `<hc-file path="outputs/report.md">Report</hc-file>`: a link to a vault
 *  file. It renders an `a[data-vault]`, which the host runtime turns into a
 *  bridge `navigate` request (the host opens the file); the kit never posts
 *  or opens anything itself. A path with a URL scheme or a network prefix is
 *  refused and shown as plain text, so this can never become a web link. */
import { baseName, vaultPath } from '../data';
import { attr, h, KitElement, part, source } from '../registry';

export class HcFile extends KitElement {
  static get observedAttributes(): string[] { return ['path']; }

  protected render(): void {
    this.clearParts();
    const raw = attr(this, 'path');
    const path = vaultPath(raw);
    const name = (source(this).textContent || '').trim() || baseName(raw) || 'File';
    const el = path
      ? part('a', 'hc-file-link', null, { href: '#', 'data-vault': path })
      : part('span', 'hc-file-link', null, { 'data-hc-invalid': '' });
    el.append(
      h('span', 'hc-file-icon', '▤', { 'aria-hidden': 'true' }),
      h('span', 'hc-file-name', name),
    );
    if (raw && raw !== name) el.append(h('span', 'hc-file-path', raw));
    this.prepend(el);
  }
}
