import { Show, createEffect, createSignal, on, onCleanup } from 'solid-js';
import { render } from 'solid-js/web';
import { homeDir } from '@tauri-apps/api/path';
import { api } from '../api';
import { fileHrefFromCode, fileRefFromCode, isFileWithLine, localFileFromHref } from '../chatFileLinks';
import { chipFolders, chipLabel, chipName, fileKind } from '../fileChip';
import { Icon, type IconName } from '../icons';
import { classifyImageUrl } from '../remoteImages';
import { loadRemoteImage } from '../remoteImageLoader';
import { chatConversationVersion, chatCwd, fileClick, openFileInSidePane, vaultRoot } from '../store/chat';
import { flashFooter } from '../store';
import { openWebLink } from '../store/browser';
import { copyTable, keepTableScroll, tableLabel } from '../replyTable';
import { INTERNAL_UI_CLASSES, MarkdownView } from './MarkdownView';
import { replyRichText, threadKey } from '../replyRichText';
import { latestRequest } from '../latestRequest';
import '../styles/reply-rich-text.css';

const IMAGE_LIMIT = 10 * 1024 * 1024;
const threadOpens = latestRequest();
let home: Promise<string | null> | undefined;
const osHome = () => home ??= homeDir().catch(() => { home = undefined; return null; });
const basename = (path: string) => path.split('/').pop() || path;
let activeImageReads = 0;
const imageWaiters: Array<() => void> = [];

// One queue for the renderer, including separate replies/segments. A queued
// read checks its DOM/conversation owner again before touching the backend.
async function readImage(path: string, current: () => boolean) {
  if (activeImageReads >= 2) await new Promise<void>(resolve => imageWaiters.push(resolve));
  else activeImageReads++;
  try {
    if (!current()) throw new Error('Image owner changed');
    return await api.chatPreviewFile(path);
  } finally {
    const next = imageWaiters.shift();
    if (next) next();
    else activeImageReads--;
  }
}

const blocks = new WeakMap<Element, { keys: string[]; nodes: ChildNode[] }>();

// File chips (2026-11-10): a file link shows its name and a type icon, not
// its path. Each icon is drawn once, into a box (a component's return value
// is an accessor in a dev build, not a node), and cloned.
const iconNodes = new Map<IconName, Element>();
function iconNode(name: IconName): Element {
  let node = iconNodes.get(name);
  if (!node) {
    const box = document.createElement('span');
    const dispose = render(() => <Icon name={name} size={14} class="file-chip-icon" />, box);
    node = box.firstElementChild!.cloneNode(true) as Element;
    dispose();
    iconNodes.set(name, node);
  }
  return node.cloneNode(true) as Element;
}

function chipPart(cls: string, text: string): HTMLElement {
  const el = document.createElement('span');
  el.className = cls; el.textContent = text;
  return el;
}

/** Turns a file link (`code.file-ref`, a prose path, a local `<a href>`) into
 *  a chip: the type icon, the file's name (or the link's own words, `label`),
 *  any cited lines, and a folder slot `chipFoldersIn` fills when two files in
 *  the reply share a name. The path as written stays its tooltip, its
 *  accessible name and what a copy takes. */
export function chipify(el: HTMLElement, path: string, label?: string): void {
  const where = path.trim();
  if (!where || el.classList.contains('file-chip')) return;
  const { name, lines } = chipName(where);
  const kind = fileKind(where);
  el.classList.add('file-chip', `tone-${kind.tone}`);
  el.dataset.chipPath = where;
  const parts: Node[] = [iconNode(kind.icon), chipPart('file-chip-name', label ?? name)];
  if (lines && !label) parts.push(chipPart('file-chip-lines', lines));
  if (!label) parts.push(chipPart('file-chip-folder', ''));
  el.replaceChildren(...parts);
  el.title = where;
  el.setAttribute('aria-label', label ? `Open ${label}, ${where}` : chipLabel(where));
}

/** Fills each named chip's folder slot in `scope` (a whole reply): empty
 *  unless another chip there has the same name and another path. */
export function chipFoldersIn(scope: Element): void {
  const slots = [...scope.querySelectorAll<HTMLElement>('.file-chip > .file-chip-folder')];
  const folders = chipFolders(slots.map(s => (s.parentElement as HTMLElement).dataset.chipPath ?? ''));
  slots.forEach((slot, i) => { if (slot.textContent !== folders[i]) slot.textContent = folders[i]; });
}

/** The path a local link names, as a chip shows it: no `file://`, decoded,
 *  no query, and only a `#L12` fragment kept. */
function linkPath(href: string): string {
  let ref = href.trim().replace(/^file:\/\/(localhost)?/i, '').replace(/\?[^#]*/, '').replace(/#(?!L\d)[^]*$/, '');
  try { ref = decodeURIComponent(ref); } catch { /* keep it as written */ }
  return ref;
}

/** A copy that takes in a file chip takes its path, not its name. */
function copyChipPaths(e: ClipboardEvent): void {
  const sel = document.getSelection();
  if (!sel || sel.rangeCount === 0 || sel.isCollapsed || !e.clipboardData) return;
  const range = sel.getRangeAt(0);
  const at = range.commonAncestorContainer;
  const inside = (at instanceof Element ? at : at.parentElement)?.closest<HTMLElement>('.file-chip');
  if (inside) { e.preventDefault(); e.clipboardData.setData('text/plain', inside.dataset.chipPath ?? ''); return; }
  const frag = range.cloneContents();
  const chips = frag.querySelectorAll<HTMLElement>('.file-chip');
  if (!chips.length) return;
  for (const chip of chips) chip.replaceWith(document.createTextNode(chip.dataset.chipPath ?? chip.textContent ?? ''));
  // Laid out off screen for a moment, so the text keeps its line breaks.
  const box = document.createElement('div');
  box.style.cssText = 'position:fixed;left:-9999px;top:0;white-space:pre-wrap';
  box.append(frag);
  document.body.append(box);
  const text = box.innerText;
  box.remove();
  e.preventDefault();
  e.clipboardData.setData('text/plain', text);
  e.clipboardData.setData('text/html', box.innerHTML);
}

/** Puts a reply's HTML in its view block by block (overnight 11-07, D1, audit
 *  gap 8): each leading top-level node whose markup is the same as last time
 *  stays mounted, and only the nodes from the first change on are replaced.
 *  A streaming reply re-renders about every 80 ms and only its tail grows, so
 *  a finished paragraph keeps its selection, a table its focus, scroll and
 *  "Copied" label, and a decorated image its picture. The result is the same
 *  DOM a whole `innerHTML` swap gives (the markup is compared as parsed, not
 *  as decorated). Anything else that changed the view's top level starts it
 *  over. */
export function keepBlocks(root: HTMLElement, html: string): void {
  const template = document.createElement('template');
  template.innerHTML = html;
  const fresh = [...template.content.childNodes];
  const keys = fresh.map(node => (node instanceof Element ? node.outerHTML : node.textContent ?? ''));
  const was = blocks.get(root);
  let same = 0;
  if (was && root.childNodes.length === was.nodes.length && was.nodes.every(node => node.parentNode === root)) {
    while (same < keys.length && same < was.keys.length && keys[same] === was.keys[same]) same++;
    for (const node of was.nodes.slice(same)) node.remove();
    root.append(...fresh.slice(same));
  } else root.replaceChildren(...fresh);
  blocks.set(root, { keys, nodes: [...(was?.nodes.slice(0, same) ?? []), ...fresh.slice(same)] });
}

/** Reply-only decoration. Other Markdown hosts retain their own image policy. */
export default function ReplyMarkdown(props: { text: string; class?: string; scroll?: number[] }) {
  const [count, setCount] = createSignal(0);
  const [loading, setLoading] = createSignal(false);
  const [loadResult, setLoadResult] = createSignal('');
  let root: HTMLElement | undefined;
  // Where each table was scrolled. A growing segment remounts this whole
  // component, so ChatMessage owns the offsets and hands them in (`scroll`);
  // a host that doesn't gets one that lives as long as this mount.
  const ownScroll: number[] = [];
  let bulkGeneration = 0;
  let alive = true;
  const images = new Map<string, Promise<string>>();
  // Local image buttons already filled in, each with the folder and conversation
  // it was filled for: a block kept by `keepBlocks` keeps them, so a later pass
  // for the same folder and conversation leaves them alone, and a pass for
  // another one fills them again (the stamp says so, not an effect that clears
  // a set: that effect and MarkdownView's decoration both run on such a change
  // and Solid doesn't promise their order, which left a kept picture with the
  // old pass's dead click). A failed one is not here, so the next pass tries it
  // again (the file may be written later in the turn): a flush is a pass, and so
  // is the turn's end (`ChatMessage` hands out new segment objects then, and
  // `decorate` reads `props.text` through them).
  const placed = new WeakMap<Element, { cwd: ReturnType<typeof chatCwd>; owner: number }>();
  createEffect(on(() => [chatCwd(), chatConversationVersion()], () => {
    // A new conversation or folder resolves every picture again, kept or not
    // (the stamps above), so what was read goes too.
    images.clear(); bulkGeneration++; setLoading(false); setLoadResult('');
  }, { defer: true }));
  onCleanup(() => { alive = false; bulkGeneration++; images.clear(); });

  function transform(html: string) {
    // Changing the owner/base invalidates every pending decoration, even if
    // the Markdown string happens to be identical in the new conversation.
    chatCwd(); chatConversationVersion();
    const template = document.createElement('template');
    template.innerHTML = html;
    // Defense in depth: strip any smuggled data-* attributes or Hollow-internal
    // UI classes before attaching legitimate local-image buttons and tables.
    for (const node of template.content.querySelectorAll('*')) {
      for (const attr of [...node.attributes]) {
        if (attr.name.toLowerCase().startsWith('data-')) node.removeAttribute(attr.name);
      }
      if (node.classList.length) {
        for (const cls of [...node.classList]) {
          if (INTERNAL_UI_CLASSES.has(cls.toLowerCase())) node.classList.remove(cls);
        }
        if (!node.classList.length) node.removeAttribute('class');
      }
    }
    for (const img of template.content.querySelectorAll('img')) {
      const src = img.getAttribute('src') ?? '';
      if (!src || /^(data|blob):/i.test(src)) continue;
      const kind = classifyImageUrl(src);
      if (kind.kind === 'remote') continue;
      if (kind.kind === 'blocked' && !/^file:\/\//i.test(src)) continue;
      const button = document.createElement('button');
      button.type = 'button'; button.className = 'reply-local-image';
      button.dataset.localImage = src;
      button.dataset.localAlt = img.getAttribute('alt') ?? '';
      button.textContent = `Loading image: ${basename(src)}`;
      button.disabled = true;
      img.replaceWith(button);
      // A thumbnail owns its click, even if the Markdown put it in a link.
      const anchor = button.closest('a');
      if (anchor) { const span = document.createElement('span'); span.append(...anchor.childNodes); anchor.replaceWith(span); }
    }
    // A table scrolls inside its own box instead of pushing the pane sideways,
    // and a Copy table button sits above it (overnight 11-07, D1). Built here,
    // not in `decorate`, so the markup is part of the HTML the view sets.
    const tables = [...template.content.querySelectorAll('table')];
    tables.forEach((table, i) => {
      // Short cells stay on one line; only a long one wraps (see `.long`).
      for (const cell of table.querySelectorAll('td, th')) if ((cell.textContent ?? '').length > 60) cell.classList.add('long');
      const box = document.createElement('div'); box.className = 'reply-table';
      const bar = document.createElement('div'); bar.className = 'reply-table-bar';
      const copy = document.createElement('button');
      copy.type = 'button'; copy.className = 'reply-table-copy'; copy.textContent = 'Copy table';
      if (tables.length > 1) copy.setAttribute('aria-label', tableLabel('Copy table', i, tables.length));
      const scroll = document.createElement('div'); scroll.className = 'reply-table-scroll';
      // Focusable, so the keyboard can scroll a wide one.
      scroll.tabIndex = 0; scroll.setAttribute('role', 'region'); scroll.setAttribute('aria-label', tableLabel('Table', i, tables.length));
      table.replaceWith(box);
      scroll.append(table); bar.append(copy); box.append(bar, scroll);
    });
    return template.innerHTML;
  }

  // One revert timer per button: a second copy inside the Copied window must
  // keep its label for its own full time, not lose it to the first copy's timer.
  const labelTimers = new WeakMap<Element, ReturnType<typeof setTimeout>>();
  async function copyFrom(button: Element) {
    const table = button.closest('.reply-table')?.querySelector('table');
    if (!table) return;
    const ok = await copyTable(table);
    flashFooter(ok ? 'Copied table' : '× copy failed', ok ? 1000 : 1500);
    if (!ok) return;
    button.textContent = 'Copied';
    clearTimeout(labelTimers.get(button));
    labelTimers.set(button, setTimeout(() => { if (button.isConnected) button.textContent = 'Copy table'; }, 1200));
  }

  // A read is cached per folder and conversation version as well as per file: a
  // pass for another one mustn't reuse a read still queued for an older one,
  // which rejects when its turn comes (`owned()` is false for it) and would take
  // the new pass's picture down with it. The cache is cleared on such a change
  // too, but that effect and the decoration aren't ordered.
  function imageData(path: string, pass: { cwd: ReturnType<typeof chatCwd>; owner: number }, current: () => boolean) {
    const key = JSON.stringify([pass.cwd, pass.owner, path]);
    const existing = images.get(key);
    if (existing) return existing;
    const pending = readImage(path, current).then(file => {
      if (file.kind !== 'image' || !file.dataUrl?.startsWith('data:image/') || file.size > IMAGE_LIMIT
        || file.dataUrl.length > Math.ceil(IMAGE_LIMIT / 3) * 4 + 128) throw new Error('Image unavailable');
      return file.dataUrl;
    });
    images.set(key, pending);
    while (images.size > 12) images.delete(images.keys().next().value!);
    void pending.catch(() => { if (images.get(key) === pending) images.delete(key); });
    return pending;
  }

  function decorate(el: HTMLElement) {
    root = el;
    keepTableScroll(el, props.scroll ?? ownScroll);
    const cwd = chatCwd(), owner = chatConversationVersion();
    // What this pass fills pictures for: their stamp, and their read cache's key.
    const stamp = { cwd, owner };
    let active = true;
    // `owned`: this view and conversation still hold; `current` adds "and this
    // is still the latest pass". A cached image read is shared by later
    // passes (the next flush is about 80 ms away), so it checks `owned`: under
    // `current` a read still queued at the next flush would reject, and the
    // pass that reused its promise would mark the image unavailable for good.
    const owned = () => el.isConnected && cwd === chatCwd() && owner === chatConversationVersion();
    const current = () => active && owned();
    const recount = () => setCount(el.querySelectorAll('button.md-remote-img[data-image-url]:not(:disabled)').length);
    recount();
    const observer = new MutationObserver(recount);
    observer.observe(el, { subtree: true, attributes: true, attributeFilter: ['disabled'] });
    // Chips are a promise (2026-11-11): one failing step logs and the next
    // still runs, so a reply's paths never stay plain code over one bad link.
    const guarded = (what: string, fn: () => void) => { try { fn(); } catch (e) { console.error(`file chips: ${what}`, e); } };
    guarded('links', () => { for (const link of el.querySelectorAll<HTMLAnchorElement>('a[href], area[href]')) {
      const href = link.getAttribute('href')!;
      if (threadKey(href)) {
        link.classList.add('reply-thread-chip');
        link.title = `Open conversation ${threadKey(href)}`;
        continue;
      }
      if (/^https?:/i.test(href)) { link.title = `${href}\n⌘-click to open in your default browser`; continue; }
      // A local file link is a chip too (2026-11-10); its own words stay
      // when they aren't a path.
      const scheme = isFileWithLine(href) ? null : /^([a-z][a-z0-9+.-]*):/i.exec(href)?.[1]?.toLowerCase();
      if (link.tagName !== 'A' || href.startsWith('#') || (scheme && scheme !== 'file') || link.querySelector('img, button')) continue;
      const path = linkPath(href), words = (link.textContent ?? '').trim();
      const named = !words || words === href || words === path || words.includes('/') || !!fileRefFromCode(words);
      chipify(link, path, named ? undefined : words);
    } });
    // Marking decorates `el` alone, so it needs only this pass to be the
    // latest (`active`), not `current()`: a view still detached when its pass
    // runs (the lazy chunk resolving inside Suspense) or a conversation that
    // changed meanwhile would otherwise leave every path plain code, with no
    // later pass to fix it (Main chat, 2026-11-11).
    function mark(homePath: string | null) {
      if (!active) return;
      guarded('paths', () => markCodes(homePath));
      guarded('prose', () => { void import('../proseFileRefs').then(m => { if (active) { m.markProsePaths(el, homePath); chipAll(); } }).catch(e => console.error('file chips: prose', e)); });
    }
    function markCodes(homePath: string | null) {
      for (const code of el.querySelectorAll<HTMLElement>('code')) {
        if (code.closest('pre, a') || code.dataset.fileRef !== undefined) continue;
        // The cited lines ride along as `#L12-20` (S1b); the text stays as written.
        const ref = fileHrefFromCode(code.textContent ?? '', homePath);
        if (!ref) continue;
        code.dataset.fileRef = ref; code.classList.add('file-ref');
        code.setAttribute('role', 'button'); code.tabIndex = 0;
        code.title = 'Preview';
      }
      chipAll();
    }
    // Every file reference becomes a chip of its name (2026-11-10), and the
    // reply's same-name chips show their folders.
    function chipAll() {
      for (const ref of el.querySelectorAll<HTMLElement>('.file-ref:not(.file-chip)')) chipify(ref, ref.textContent ?? '');
      chipFoldersIn(el.closest('.chat-msg') ?? el);
    }
    el.addEventListener('copy', copyChipPaths);
    mark(null);
    if (props.text.includes('~/')) void osHome().then(mark);
    const buttons = [...el.querySelectorAll<HTMLButtonElement>('button.reply-local-image[data-local-image]')].filter(button => {
      const at = placed.get(button);
      return !(at && at.cwd === cwd && at.owner === owner);
    });
    let index = 0;
    // A picture that loaded for another folder or conversation and fails in
    // this one is a failed picture again: nothing to open.
    const failed = (button: HTMLButtonElement, text: string) => {
      // Its old stamp goes too, or coming back to that folder would skip it.
      placed.delete(button);
      button.disabled = true; button.removeAttribute('aria-label'); button.onclick = null;
      button.textContent = text;
    };
    async function worker() {
      const base = cwd || await vaultRoot();
      while (current() && index < buttons.length) {
        const button = buttons[index++], ref = button.dataset.localImage!;
        let path: string | null = null;
        try {
          const homePath = ref.includes('~') ? await osHome() : null;
          if (!current()) return;
          path = localFileFromHref(ref, base, homePath);
          if (!path) throw new Error('Image path unavailable');
          const data = await imageData(path, stamp, owned);
          if (!current() || !el.contains(button)) return;
          const img = document.createElement('img'); img.src = data; img.alt = button.dataset.localAlt || basename(path);
          button.replaceChildren(img); button.disabled = false;
          button.setAttribute('aria-label', `Expand ${basename(path)}`);
          const target = path;
          // A kept block outlives this pass: the click checks the view's
          // conversation, not whether this pass is the latest.
          button.onclick = e => {
            e.preventDefault(); e.stopPropagation();
            if (owned()) void import('./ImageZoom').then(({ showImageZoom }) => {
              if (owned()) showImageZoom(data, img.alt, target);
            });
          };
          placed.set(button, stamp);
        } catch (error) {
          if (!current() || !el.contains(button)) return;
          const missing = /no such file|not found/i.test(error instanceof Error ? error.message : String(error));
          failed(button, `Image ${missing ? 'not found' : 'unavailable'}: ${basename(path ?? ref)}`);
        }
      }
    }
    if (buttons.length) {
      const failedBase = () => {
        if (current()) for (const button of buttons) failed(button, `Image unavailable: ${basename(button.dataset.localImage!)}`);
      };
      void worker().catch(failedBase); void worker().catch(failedBase);
    }
    return () => { active = false; observer.disconnect(); el.removeEventListener('copy', copyChipPaths); };
  }

  async function loadAll() {
    if (!root || loading()) return;
    const urls = [...new Set([...root.querySelectorAll<HTMLButtonElement>('button.md-remote-img[data-image-url]')]
      .filter(b => !b.disabled).map(b => b.dataset.imageUrl!))];
    const mine = ++bulkGeneration, owner = chatConversationVersion();
    setLoading(true); setLoadResult('');
    // A bounded queue shares the existing request coalescer and cache.
    let next = 0, failed = 0;
    const worker = async () => {
      while (alive && next < urls.length && owner === chatConversationVersion()) {
        try { await loadRemoteImage(urls[next++]); } catch { failed++; }
      }
    };
    await Promise.all([worker(), worker()]);
    if (mine !== bulkGeneration || owner !== chatConversationVersion()) return;
    setLoading(false);
    setLoadResult(failed ? `${failed} image${failed === 1 ? '' : 's'} could not load. Try again.` : 'Images loaded.');
  }

  function open(e: MouseEvent | KeyboardEvent) {
    const target = e.target as Element | null;
    // A table's Copy button rides in the HTML, so its click arrives here.
    const copy = e.type === 'click' ? target?.closest?.('button.reply-table-copy') : null;
    if (copy) { e.preventDefault(); e.stopPropagation(); void copyFrom(copy); return; }
    // A backticked path or (2026-11-04) one written in the prose.
    const code = target?.closest?.('.file-ref') as HTMLElement | null;
    const link = code ? null : target?.closest?.('a[href], area[href]');
    const ref = code?.dataset.fileRef ?? link?.getAttribute('href') ?? '';
    if (!ref || ref.startsWith('#')) return;
    e.preventDefault(); e.stopPropagation();
    if (/^https?:/i.test(ref)) { openWebLink(ref, e); return; }
    const key = threadKey(ref);
    if (key) {
      const version = chatConversationVersion();
      const request = threadOpens.start();
      // Navigation may intentionally unmount this reply (Library to Today).
      // Its request, rather than its DOM lifetime, owns the subsequent reads.
      const current = () => threadOpens.isLatest(request) && version === chatConversationVersion();
      void import('../store/workspaces').then(({ openSessionByKey }) => {
        if (alive && current()) return openSessionByKey(key, current);
      }).catch(() => { if (current()) flashFooter('Hollow could not open this conversation.'); });
      return;
    }
    // `README.md:12` is a file and its line, not a `README.md:` scheme.
    const scheme = isFileWithLine(ref) ? null : /^([a-z][a-z0-9+.-]*):/i.exec(ref)?.[1];
    if (scheme && scheme.toLowerCase() !== 'file') { flashFooter(`Hollow can't open '${scheme}:' links`); return; }
    // The click's conversation and its ask of the pane, taken now.
    const click = fileClick();
    void (async () => {
      const homePath = ref.includes('~') ? await osHome() : null;
      if (click.version !== chatConversationVersion() || click.cwd !== chatCwd()) return;
      // Every folder the chat works in is tried (2026-11-04), and a cited
      // `path:12` / `path:12-20` / `path#L12` opens at those lines.
      const { openNamedFile } = await import('../store/fileOpen');
      await openNamedFile(ref, homePath, { owner: click });
    })().catch(() => flashFooter('Hollow could not open this file.'));
  }

  return <>
    <MarkdownView text={props.text} parse={replyRichText} class={props.class} decorate={decorate} transform={transform} apply={keepBlocks} onLinkClick={open} replyFiles forbidMedia />
    <Show when={count() > 0}>
      <div class="reply-image-controls">
        <span>Remote images stay blocked until you load them.</span>
        <button type="button" class="btn small" disabled={loading()} onClick={() => void loadAll()}>
          {loading() ? 'Loading images…' : `Load ${count()} image${count() === 1 ? '' : 's'}`}
        </button>
      </div>
    </Show>
    <Show when={loadResult()}><p role="status">{loadResult()}</p></Show>
  </>;
}
