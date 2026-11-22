/** `<hc-code lang="ts" label="htmlSandbox.ts">const x = 1;</hc-code>`: a
 *  code block. The text is the author's (escape `<` as `&lt;`); it is
 *  dedented and lightly highlighted with the `--app-syn-*` tokens, every
 *  piece written as text. The block scrolls sideways and is focusable, so a
 *  keyboard can scroll it too. */
import { dedent, highlight } from '../data';
import { attr, h, KitElement, part, source } from '../registry';

export class HcCode extends KitElement {
  static get observedAttributes(): string[] { return ['lang', 'label']; }

  protected render(): void {
    this.clearParts();
    const lang = attr(this, 'lang');
    const label = attr(this, 'label');
    const text = dedent(source(this).textContent || '');
    const parts: HTMLElement[] = [];
    if (label || lang) {
      const head = part('div', 'hc-code-head');
      if (label) head.append(h('span', 'hc-code-label', label));
      if (lang) head.append(h('span', 'hc-code-lang', lang));
      parts.push(head);
    }
    const pre = part('pre', 'hc-code-pre', null, { tabindex: '0', 'aria-label': label ? `Code: ${label}` : 'Code' });
    const code = document.createElement('code');
    for (const [kind, t] of highlight(text, lang)) code.append(kind ? h('span', `hc-syn-${kind}`, t) : document.createTextNode(t));
    pre.append(code);
    parts.push(pre);
    this.prepend(...parts);
  }
}
