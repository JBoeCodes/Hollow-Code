/** Main-window link guard (2026-10-02, review A P2). A web or mail link
 *  clicked anywhere in the main window (a note, rendered markdown, a setting)
 *  would otherwise navigate Hollow's own window away. App.tsx listens for
 *  clicks on `window`, after every component handler has run, and takes over
 *  any link nothing else claimed: web links go where a reply's links go
 *  (`openWebLink`), mail links to the default app. Rust's main-window
 *  navigation guard (`main_nav.rs`) is the backstop for anything this
 *  misses. Pure, so it is unit-tested without a DOM. */

export type LinkKind = 'web' | 'mail';

/** What a link's `href` is, if it is one Hollow opens outside its window. */
export function linkKind(href: string | null | undefined): LinkKind | null {
  const h = (href ?? '').trim();
  if (/^https?:\/\//i.test(h)) return 'web';
  if (/^mailto:/i.test(h)) return 'mail';
  return null;
}

const EDITABLE = '[contenteditable]:not([contenteditable="false"])';

interface ClickLike {
  defaultPrevented: boolean;
  button: number;
  target: EventTarget | null;
}

interface Closest {
  closest?: (sel: string) => { getAttribute(name: string): string | null } | null;
}

/** The link a click would follow out of the window, or null to leave the
 *  click alone: another handler already took it, it isn't the primary
 *  button, it is inside an editor, or it isn't on a web/mail link. In-page
 *  anchors and app-relative links keep their default. */
export function linkToTakeOver(e: ClickLike): { href: string; kind: LinkKind } | null {
  if (e.defaultPrevented || e.button !== 0) return null;
  const target = e.target as unknown as Closest | null;
  // Inside an editor a link is text being edited; WebKit doesn't follow it.
  if (target?.closest?.(EDITABLE)) return null;
  const link = target?.closest?.('a[href]');
  const href = link?.getAttribute('href')?.trim() ?? '';
  const kind = linkKind(href);
  return kind ? { href, kind } : null;
}

/** Whether to honour a frame's open request. An Interactive artifact runs its
 *  own scripts and can post the same message from a timer, with no click.
 *  Main's `navigator.userActivation` is no guard on its own: any input in
 *  the main window (typing in the composer, a click anywhere in Hollow)
 *  activates it, and that activation lasts seconds. So the request must come
 *  while the user is interacting with that frame: `focused` is whether the
 *  sending iframe is main's `document.activeElement` (clicking inside an
 *  iframe focuses it; typing in the composer moves focus away). `active` is
 *  main's transient activation, also required where the WebView reports it
 *  (undefined where it lacks `navigator.userActivation`). And at most
 *  `limit` opens per `windowMs` either way, so a script piggybacking on the
 *  user's clicks inside the frame can't open a page per click for long. */
export class FrameOpenGate {
  private opened: number[] = [];
  constructor(private readonly limit = 3, private readonly windowMs = 60_000) {}

  admit(now: number, focused: boolean, active: boolean | undefined): boolean {
    if (!focused || active === false) return false;
    this.opened = this.opened.filter(t => now - t < this.windowMs);
    if (this.opened.length >= this.limit) return false;
    this.opened.push(now);
    return true;
  }
}
