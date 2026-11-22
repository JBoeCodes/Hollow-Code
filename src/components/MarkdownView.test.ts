import { describe, expect, it } from 'vitest';
import { blockRemoteImages } from '../remoteImages';
import markdownViewSource from './MarkdownView.tsx?raw';
import replyMarkdownSource from './ReplyMarkdown.tsx?raw';

describe('MarkdownView & ReplyMarkdown untrusted HTML hardening', () => {
  it('disables data-* attributes in DOMPurify and strips internal UI classes before transform/blockRemoteImages', () => {
    expect(markdownViewSource).toContain('ALLOW_DATA_ATTR: false');
    expect(markdownViewSource).toContain('stripInternalUiClasses(clean)');
    for (const cls of ['file-ref', 'md-remote-img', 'reply-local-image']) {
      expect(markdownViewSource).toContain(`'${cls}'`);
    }
  });

  it('strips spoofed data-* attributes and internal UI classes in ReplyMarkdown.transform before attaching legitimate buttons', () => {
    const transformIdx = replyMarkdownSource.indexOf('function transform(html: string)');
    const stripIdx = replyMarkdownSource.indexOf('INTERNAL_UI_CLASSES.has', transformIdx);
    const imgLoopIdx = replyMarkdownSource.indexOf("template.content.querySelectorAll('img')", transformIdx);
    expect(transformIdx).toBeGreaterThan(-1);
    expect(stripIdx).toBeGreaterThan(transformIdx);
    expect(stripIdx).toBeLessThan(imgLoopIdx);
    expect(replyMarkdownSource).toContain("button.reply-local-image[data-local-image]");
  });

  it('neutralizes spoofed file-ref, md-remote-img, and reply-local-image payloads while preserving legitimate markup', () => {
    // Exercise the exact stripInternalUiClasses implementation from MarkdownView.tsx
    // alongside simulated ALLOW_DATA_ATTR: false stripping and blockRemoteImages.
    const start = markdownViewSource.indexOf('export const INTERNAL_UI_CLASSES');
    const end = markdownViewSource.indexOf('export function MarkdownView');
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const body = markdownViewSource
      .slice(start, end)
      .replace(/\bexport\s+/g, '')
      .replace(/:\s*(?:string \| undefined|string \| null|string|boolean)/g, '');
    const stripInternalUiClasses = new Function(`${body}\nreturn stripInternalUiClasses;`)() as (html: string) => string;
    const stripDataAttrs = (html: string) => html.replace(/\s+data-[a-z0-9-]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?/gi, '');
    const sanitize = (html: string) => stripInternalUiClasses(stripDataAttrs(html));

    // 1. Spoofed <code class="file-ref" data-file-ref="...">
    const spoofedFileRef = sanitize('<code class="file-ref" data-file-ref="/etc/passwd">click me</code>');
    expect(spoofedFileRef).toBe('<code>click me</code>');
    expect(sanitize('<code class="file-ref language-ts" data-file-ref="/etc/passwd">click me</code>'))
      .toBe('<code class="language-ts">click me</code>');
    expect(sanitize('<code class="file&#x2d;ref" data-file-ref="/etc/passwd">entity spoof</code>'))
      .toBe('<code>entity spoof</code>');

    // 2. Spoofed <button class="md-remote-img" data-image-url="...">
    const spoofedRemoteImg = sanitize('<button class="md-remote-img" data-image-url="https://evil.example/track.png">Load</button>');
    expect(spoofedRemoteImg).toBe('<button>Load</button>');
    expect(spoofedRemoteImg).not.toContain('md-remote-img');
    expect(spoofedRemoteImg).not.toContain('data-image-url');

    // 3. Spoofed <button class="reply-local-image" data-local-image="...">
    const spoofedLocalImg = sanitize('<button class="reply-local-image" data-local-image="/etc/passwd">fake</button>');
    expect(spoofedLocalImg).toBe('<button>fake</button>');
    expect(spoofedLocalImg).not.toContain('reply-local-image');
    expect(spoofedLocalImg).not.toContain('data-local-image');

    // 4. Legitimate remote images still get placeholders from blockRemoteImages AFTER sanitization
    const legitRemote = blockRemoteImages(sanitize('<p><img src="https://example.com/chart.png" alt="chart"></p>'));
    expect(legitRemote).toContain('class="md-remote-img"');
    expect(legitRemote).toContain('data-image-url="https://example.com/chart.png"');

    // 5. Legitimate reply-rich-text classes and plain text containing class strings stay intact
    expect(sanitize('<aside class="reply-alert reply-alert-warning" role="note"><strong>Warning</strong></aside>'))
      .toBe('<aside class="reply-alert reply-alert-warning" role="note"><strong>Warning</strong></aside>');
    expect(sanitize('<div class="reply-math" role="math">x</div>'))
      .toBe('<div class="reply-math" role="math">x</div>');
    expect(sanitize('<p>class="file-ref md-remote-img reply-local-image"</p>'))
      .toBe('<p>class="file-ref md-remote-img reply-local-image"</p>');
  });
});
