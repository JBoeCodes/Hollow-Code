import { createEffect, createMemo } from 'solid-js';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { WIKILINK_RE, stripFrontmatter } from '../utils';
import { NOTE_MARKDOWN } from '../markdownOptions';
import { blockRemoteImages } from '../remoteImages';
import { handleRemoteImageClick, loadedRemoteImage, remoteImagesVersion } from '../remoteImageLoader';

/**
 * Renders a markdown note as HTML. Frontmatter is stripped before parsing
 * (the FrontmatterPanel handles it separately). Wikilinks `[[Name]]`
 * become clickable anchors that fire `onWikilinkClick`.
 *
 * The output is dropped into a `.viewer-rendered` wrapper so existing
 * `.viewer h1/h2/p/pre/code/blockquote` styles take over.
 *
 * `tasks` makes the checklist boxes tick (a workspace's PLAN.md in the Files
 * tab, COO rebuild L4): the nth box is the nth checklist row, and only when
 * the drawn boxes and the rows agree in number; otherwise they stay read-only.
 * A click asks `toggle` and leaves the box as the file says until it is read
 * again.
 */
export function NoteRenderer(props: {
  content: string;
  onWikilinkClick?: (name: string) => void;
  tasks?: { count: number; toggle: (index: number, done: boolean) => void; busy?: boolean };
}) {
  let root: HTMLDivElement | undefined;
  const sanitized = createMemo(() => {
    const content = props.content;
    if (typeof content !== 'string' || !content) return '';
    const body = stripFrontmatter(content);
    if (!body.trim()) return '<p class="hint" style="color:var(--muted-soft);font-style:italic;">(empty)</p>';
    const preprocessed = body.replace(WIKILINK_RE, (_m, name: string, alias?: string) => {
      const safeTarget = name.replace(/"/g, '&quot;');
      const text = (alias ?? name).trim();
      return `<a class="wikilink" href="#" data-name="${safeTarget}">${text}</a>`;
    });
    try {
      const raw = marked.parse(preprocessed, NOTE_MARKDOWN) as string;
      // Synced email bodies can carry arbitrary HTML — sanitize before
      // injecting via innerHTML. ADD_ATTR preserves our wikilink hooks.
      // `<style>` goes: it would restyle the app, and its url()s fetch.
      return DOMPurify.sanitize(raw, {
        USE_PROFILES: { html: true },
        ADD_ATTR: ['data-name'],
        FORBID_TAGS: ['style'],
      });
    } catch (err) {
      console.error('NoteRenderer marked parse failed:', err);
      // Fall back to plain-text rendering so the body never just disappears.
      const escaped = preprocessed
        .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
      return `<pre>${escaped}</pre>`;
    }
  });
  // Remote images are click-to-load placeholders (merged plan slice 1, review
  // A·S3): a synced page or web clip must not fetch just by being opened.
  // Its own memo so a finished load re-runs only this cheap rewrite.
  const html = createMemo(() => {
    remoteImagesVersion();
    return blockRemoteImages(sanitized(), loadedRemoteImage);
  });

  createEffect(() => {
    html();
    const tasks = props.tasks;
    const boxes = Array.from(root?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]') ?? []);
    const live = !!tasks && boxes.length === tasks.count;
    boxes.forEach((box, i) => {
      box.disabled = !live || !!tasks?.busy;
      if (live) {
        box.dataset.task = String(i);
        box.setAttribute('aria-label', box.parentElement?.textContent?.trim() || `Row ${i + 1}`);
      } else delete box.dataset.task;
    });
  });

  function handleClick(e: MouseEvent) {
    if (handleRemoteImageClick(e)) return;
    const target = e.target as HTMLElement | null;
    if (!target) return;
    if (target instanceof HTMLInputElement && target.dataset.task && props.tasks) {
      // The box shows what the file says: it changes once PLAN.md is read again.
      e.preventDefault();
      props.tasks.toggle(Number(target.dataset.task), target.checked);
      return;
    }
    const wl = target.closest?.('a.wikilink') as HTMLAnchorElement | null;
    if (wl) {
      e.preventDefault();
      props.onWikilinkClick?.(wl.dataset.name || wl.textContent || '');
    }
  }

  return (
    <div
      ref={root}
      class="viewer-rendered"
      onClick={handleClick}
      // SolidJS reactively patches innerHTML when html() changes.
      innerHTML={html()}
    />
  );
}

