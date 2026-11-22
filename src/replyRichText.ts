import { Marked, type Tokens } from 'marked';
import katex from 'katex';
import { markdownOptions } from './markdownOptions';

const parser = new Marked();
const math = (text: string, displayMode: boolean) => katex.renderToString(text, {
  displayMode, output: 'mathml', trust: false, strict: 'ignore', throwOnError: false,
  maxExpand: 1000, maxSize: 20,
});

parser.use({ extensions: [
  {
    name: 'displayMath', level: 'block',
    start: src => src.indexOf('$$'),
    tokenizer(src) {
      const m = /^\$\$[ \t]*\n?([\s\S]{1,8000}?)\n?\$\$(?:[ \t]*\n|$)/.exec(src);
      if (m) return { type: 'displayMath', raw: m[0], text: m[1] };
    },
    renderer: token => `<div class="reply-math" role="math" aria-label="${escapeAttribute(token.text)}">${math(token.text, true)}</div>`,
  },
  {
    name: 'inlineMath', level: 'inline',
    start: src => src.indexOf('$'),
    tokenizer(src) {
      const m = /^\$(?!\$|\s)([^\n$]{1,2000}?)\$(?!\d)/.exec(src);
      if (m && !/\s$/.test(m[1])) return { type: 'inlineMath', raw: m[0], text: m[1] };
    },
    renderer: token => `<span role="math" aria-label="${escapeAttribute(token.text)}">${math(token.text, false)}</span>`,
  },
], renderer: {
  blockquote(token: Tokens.Blockquote) {
    const match = /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\](?:\n|$)/.exec(token.text);
    if (!match) return false;
    const title = match[1][0] + match[1].slice(1).toLowerCase();
    return `<aside class="reply-alert reply-alert-${match[1].toLowerCase()}" role="note"><strong>${title}</strong>${this.parser.parse(parser.lexer(token.text.slice(match[0].length)))}</aside>`;
  },
} });

function escapeAttribute(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/** Raw HTML must still pass through MarkdownView's existing sanitizer. */
export function replyRichText(text: string, breaks: boolean): string {
  return parser.parse(text, markdownOptions(breaks)) as string;
}

/** Only an exact local conversation route, never a generic deep-link action. */
export function threadKey(href: string): string | null {
  const m = /^hollow:\/\/conversation\/([a-zA-Z0-9][a-zA-Z0-9_-]{0,199})$/.exec(href);
  return m?.[1] ?? null;
}
