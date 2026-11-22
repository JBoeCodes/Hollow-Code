/** Address-field rules for the Preview pane's web mode (BrowserPane.tsx).
 *  Pure, so they're unit-tested; the Rust side (browser_pane.rs) re-checks
 *  every address before a tab loads it. */
import { localFileFromHref } from './chatFileLinks';

const SEARCH = 'https://www.google.com/search?q=';
const LOCAL_HOST = /^(localhost|127(?:\.\d{1,3}){3}|0\.0\.0\.0|\[::1\]|[\w-]+\.local)(:\d{1,5})?(\/.*)?$/i;
const IPV4 = /^\d{1,3}(?:\.\d{1,3}){3}(:\d{1,5})?(\/.*)?$/;
/** host.tld[:port][/path]: a dot, no spaces, a plausible last label. */
const HOST = /^[\w-]+(\.[\w-]+)*\.[a-z][a-z0-9-]*(:\d{1,5})?([/?#].*)?$/i;

/** What typing `input` and pressing Enter loads: a web address as typed, a
 *  bare host with a scheme added (http for this Mac and the LAN, https
 *  otherwise), a bare port as localhost, and anything else as a search.
 *  Null for an empty field or a scheme the browser won't open. */
export function normalizeAddress(input: string): string | null {
  const text = input.trim();
  if (!text) return null;
  if (/^https?:\/\//i.test(text)) return text;
  if (/^about:blank$/i.test(text)) return 'about:blank';
  if (/^:?\d{2,5}$/.test(text)) return `http://localhost:${text.replace(':', '')}`;
  if (/^[a-z][a-z0-9+.-]*:(?!\d)/i.test(text) && !LOCAL_HOST.test(text)) return null;
  if (/\s/.test(text)) return SEARCH + encodeURIComponent(text);
  if (LOCAL_HOST.test(text) || IPV4.test(text)) return `http://${text}`;
  if (HOST.test(text)) return `https://${text}`;
  return SEARCH + encodeURIComponent(text);
}

/** A file typed into the address field (a `file:` address or an absolute
 *  path): the Preview pane shows it read only in its file mode, since a web
 *  tab never loads files (run 15 P2). Null for anything else, a network
 *  path included. */
export function addressFile(input: string): string | null {
  const text = input.trim();
  if (!/^file:\/\//i.test(text) && !(text.startsWith('/') && !text.startsWith('//'))) return null;
  if (/^file:\/\/(?!localhost\/|\/)/i.test(text)) return null;
  return localFileFromHref(text, null);
}

/** Whether a link should open in the Preview pane's web mode at all. */
export const isWebUrl = (url: string) => /^https?:\/\//i.test(url);

/** The tab strip's label: the page title, else the host. */
export function tabLabel(url: string, title: string): string {
  if (title.trim()) return title.trim();
  try {
    const u = new URL(url);
    return u.host || url;
  } catch { return url || 'New tab'; }
}
