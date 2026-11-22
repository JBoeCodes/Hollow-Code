import { describe, expect, it } from 'vitest';
import { marked } from 'marked';
import { blockRemoteImages, classifyImageUrl, neutralizeStyle, srcsetUrls } from './remoteImages';

/**
 * Remote images in rendered markdown are click-to-load placeholders (merged
 * plan slice 1, reviews A·S3 / B·F13). In the app the rewrite runs on
 * DOMPurify's output; vitest has no DOM, so these feed it marked's output and
 * hand-written HTML directly — the harder case, since nothing has normalized
 * the markup first.
 */

const md = (text: string) => blockRemoteImages(marked.parse(text, { gfm: true, breaks: true, async: false }) as string);

/** Every attribute that makes the browser fetch, as (name, value) pairs. */
function fetchAttrs(html: string): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  for (const m of html.matchAll(/<[a-zA-Z][^>]*>/g)) {
    for (const a of m[0].matchAll(/[\s/](src|srcset|poster|background|lowsrc|dynsrc)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/gi)) {
      out.push([a[1].toLowerCase(), a[2] ?? a[3] ?? a[4] ?? '']);
    }
  }
  return out;
}

/** No fetching attribute in the output names anything but a local image. */
function expectNoRemoteFetch(html: string) {
  for (const [name, value] of fetchAttrs(html)) {
    const urls = name === 'srcset' ? srcsetUrls(value) : [value];
    for (const u of urls) expect(classifyImageUrl(u).kind, `${name}="${value}" in ${html}`).toBe('local');
  }
  expect(html).not.toMatch(/url\(\s*['"]?\s*(?:https?:|\/\/)/i);
}

const placeholders = (html: string) => html.match(/<button type="button" class="md-remote-img"/g)?.length ?? 0;

describe('blockRemoteImages', () => {
  it('turns a markdown image into a placeholder naming its host and alt', () => {
    const out = md('Look: ![a chart](https://e.com/a.png?q=SECRET)');
    expectNoRemoteFetch(out);
    expect(placeholders(out)).toBe(1);
    expect(out).toContain('Image from e.com — Load');
    expect(out).toContain('<span class="md-remote-img-alt">a chart</span>');
    expect(out).toContain('data-image-url="https://e.com/a.png?q=SECRET"');
    expect(out).not.toMatch(/<img/);
  });

  it.each([
    ['raw <img>', '<img src="https://e.com/a.png">'],
    ['uppercase scheme and tag', '<IMG SRC="HTTPS://E.COM/A.PNG">'],
    ['http:', '<img src="http://e.com/a.png">'],
    ['protocol-relative', '<img src="//e.com/a.png">'],
    ['backslash protocol-relative', '<img src="\\\\e.com/a.png">'],
    ['unquoted after a slash', '<img/src=https://e.com/a.png>'],
    ['single-quoted', "<img src='https://e.com/a.png'>"],
    ['quoted > before src', '<img alt="a > b" src="https://e.com/a.png">'],
    ['entity-encoded scheme', '<img src="&#104;ttps&colon;//e.com/a.png">'],
    ['tab inside the scheme', '<img src="ht&#x09;tps://e.com/a.png">'],
    ['leading whitespace', '<img src="  https://e.com/a.png">'],
    ['<image> alias', '<image src="https://e.com/a.png">'],
    ['srcset only', '<img srcset="https://e.com/a.png 1x, https://e.com/b.png 2x">'],
    ['srcset after a blocked src', '<img src="file:///x.png" srcset="https://e.com/b.png 2x">'],
  ])('%s becomes a placeholder with no remote URL left', (_name, html) => {
    const out = blockRemoteImages(`<p>${html}</p>`);
    expectNoRemoteFetch(out);
    expect(placeholders(out)).toBe(1);
    expect(out).toMatch(/Image from e\.com — Load/i);
  });

  it('strips remote sources from <picture> and keeps none in any srcset', () => {
    const out = blockRemoteImages(
      '<picture><source srcset="https://e.com/a.webp 1x, https://e.com/a2.webp 2x" type="image/webp"><source src="https://e.com/a.avif"><img src="https://e.com/a.png" alt="pic"></picture>',
    );
    expectNoRemoteFetch(out);
    expect(out).toContain('<source type="image/webp">');
    expect(placeholders(out)).toBe(1);
    expect(out).toContain('<span class="md-remote-img-alt">pic</span>');
  });

  it('drops a remote srcset from a local image and keeps the image', () => {
    const out = blockRemoteImages('<img src="data:image/png;base64,AAAA" srcset="https://e.com/b.png 2x" alt="x">');
    expectNoRemoteFetch(out);
    expect(out).toBe('<img src="data:image/png;base64,AAAA" alt="x">');
  });

  it('removes remote URLs from every other fetching attribute', () => {
    const out = blockRemoteImages(
      '<input type="image" src="https://e.com/i.png"><video poster="https://e.com/p.png" src="https://e.com/v.mp4" controls></video><table background="https://e.com/bg.png"><tr><td background="//e.com/t.png">x</td></tr></table>',
    );
    expectNoRemoteFetch(out);
    expect(out).toContain('<input type="image">');
    expect(out).toContain('<video controls="">');
    expect(placeholders(out)).toBe(0);
  });

  it('neutralizes remote url() in style attributes', () => {
    const out = blockRemoteImages(
      '<p style="color:red;background-image:url(https://e.com/bg.png)">a</p>'
      + '<div style="background: url( \'//e.com/x.png\' ) no-repeat">b</div>'
      + '<span style="background:URL(&quot;HTTPS://e.com/y.png&quot;)">c</span>',
    );
    expectNoRemoteFetch(out);
    expect(out).toContain('<p style="color:red;background-image:none">a</p>');
    expect(out).toContain('<div style="background: none no-repeat">b</div>');
  });

  it('drops style attributes it cannot read safely', () => {
    expect(neutralizeStyle('background:u\\72l(https://e.com/a.png)')).toBeNull();
    expect(neutralizeStyle('background-image:image-set("https://e.com/a.png" 1x)')).toBeNull();
    expect(neutralizeStyle('background-image:url(https://e.com/a.png')).toBeNull();
    expect(blockRemoteImages('<p style="background:u\\72l(https://e.com/a.png)">a</p>')).toBe('<p>a</p>');
  });

  it('keeps data:, blob: and relative images exactly as they were', () => {
    const local = [
      '<img src="data:image/png;base64,AAAA" alt="d">',
      '<img src="data:image/svg+xml;utf8,&lt;svg/&gt;">',
      '<img src="blob:tauri://localhost/2f1c">',
      '<img srcset="data:image/png;base64,AA,BB 1x, data:image/png;base64,CC 2x">',
      '<p style="background:url(data:image/png;base64,AAAA)">x</p>',
    ];
    for (const html of local) expect(blockRemoteImages(html)).toBe(html);
    const rel = md('![diagram](./diagram.png)');
    expect(rel).toContain('<img src="./diagram.png" alt="diagram">');
  });

  it('drops images on other schemes', () => {
    const out = blockRemoteImages('<img src="file:///etc/x.png" alt="f">');
    expect(out).toBe('<img alt="f">');
  });

  it('does not treat text inside raw-text elements or comments as tags', () => {
    const html = '<textarea><img src="https://e.com/a.png"></textarea><!-- <img src="https://e.com/b.png"> --><p>ok</p>';
    expect(blockRemoteImages(html)).toBe(html);
  });

  it('escapes the alt text and host it puts in the placeholder', () => {
    const out = blockRemoteImages('<img alt="&quot;&gt;<script>x</script>" src="https://e.com/a.png">');
    expect(out).not.toContain('<script');
    expect(out).toContain('&lt;script&gt;');
  });

  it('shows an image the user already loaded from its data: URL', () => {
    const loaded = (url: string) => (url === 'https://e.com/a.png' ? 'data:image/png;base64,AAAA' : undefined);
    const out = blockRemoteImages('<img src="https://e.com/a.png" srcset="https://e.com/b.png 2x" alt="a" width="10">', loaded);
    expect(out).toBe('<img src="data:image/png;base64,AAAA" alt="a" width="10">');
    // A cache entry that isn't an image data: URL is never trusted.
    const bad = blockRemoteImages('<img src="https://e.com/a.png">', () => 'https://e.com/a.png');
    expectNoRemoteFetch(bad);
    expect(placeholders(bad)).toBe(1);
  });
});

describe('classifyImageUrl', () => {
  it('resolves protocol-relative URLs to https on the named host', () => {
    expect(classifyImageUrl('//e.com/a.png')).toEqual({ kind: 'remote', url: 'https://e.com/a.png', host: 'e.com' });
    expect(classifyImageUrl('/\\e.com/a.png')).toMatchObject({ kind: 'remote', host: 'e.com' });
  });

  it('labels look-alike hosts in punycode', () => {
    expect(classifyImageUrl('https://ex\u0430mple.com/a.png')).toMatchObject({ kind: 'remote', host: 'xn--exmple-4nf.com' });
  });
});
