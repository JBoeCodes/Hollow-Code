import { expect, it } from 'vitest';
import { replyRichText, threadKey } from './replyRichText';

it('renders bounded math as native MathML and leaves code literal', () => {
  const rendered = replyRichText('$x^2$ and\n\n$$\n\\frac{1}{2}\n$$\n\n`$literal$`', true);
  expect(rendered).toContain('<math');
  expect(rendered).toContain('<mfrac>');
  expect(rendered).toContain('<code>$literal$</code>');
  expect(replyRichText('```text\n$x$\n```', true)).not.toContain('<math');
  expect(replyRichText('$ 10 and $20', true)).not.toContain('<math');
});

it('does not grant math commands links, images or authored HTML authority', () => {
  const rendered = replyRichText('$\\href{javascript:alert(1)}{bad}$ $\\includegraphics{https://example.com/track}$', true);
  expect(rendered).not.toMatch(/<(?:a|img)\b/);
  // This is raw parser output: MarkdownView must still sanitize authored HTML.
  expect(replyRichText('<script>bad()</script>', true)).toContain('<script>');
});

it('renders GitHub alerts and keeps ordinary blockquotes ordinary', () => {
  expect(replyRichText('> [!WARNING]\n> Keep **backups**.', true)).toContain('reply-alert-warning');
  expect(replyRichText('> [!WARNING]\n> Keep **backups**.', true)).toContain('<strong>backups</strong>');
  expect(replyRichText('> Normal quote', true)).toContain('<blockquote>');
});

it('accepts only exact conversation keys with no queries, traversal or alternate action', () => {
  expect(threadKey('hollow://conversation/s-20261120-abc')).toBe('s-20261120-abc');
  for (const ref of ['hollow://issue/ENG-1', 'hollow://conversation/../secret', 'hollow://conversation/s-a?send=x', 'hollow://conversation/s-a#x', 'hollow://conversation/%2fetc', 'https://conversation/s-a']) expect(threadKey(ref)).toBeNull();
});
