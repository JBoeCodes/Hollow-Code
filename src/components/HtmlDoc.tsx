import { Show, batch, createEffect, on, createSignal, createMemo, createResource, onMount, onCleanup, untrack } from 'solid-js';
import { lookFor } from '../store/canvasLook';
import type { Look } from '../canvasLook';
import {
  DEFAULT_ARTIFACT_MODE, SANDBOX,
  artifactDocUrl, loadCanvasKit, loadedCanvasKit, modeRunsScripts, sandboxDocParts, sealDoc, srcdocMode,
  watchFrameTheme,
  type ArtifactMode,
} from './htmlSandbox';
import type { CanvasKitAssets } from '../canvas/kitAssets';
import { acceptFrameMessage } from '../canvasBridge';
import { logBridgeRefusal } from '../canvas/bridgeRefusals';
import { openFrameLink } from '../store/browser';
import { allows, blockedLine, escalate, grantedMode, setGrant, textKey, withoutInternet } from '../canvas/canvasGrants';
import type { BlockedWant } from '../canvasBridge';

/**
 * Renders a full agent-authored HTML *document* (e.g. a Morning Brief produced
 * by a workflow) as a live artifact in the same HARD-SANDBOXED iframe as
 * `HtmlArtifact` (null origin, child CSP — see ./htmlSandbox). Differences from
 * the chat artifact:
 *   - No 640px cap and no Preview/Code chrome — the brief sizes to its content
 *     and flows with the page (the page-body scrolls, not a nested frame).
 *   - A vault-link BRIDGE: a click on `<a data-vault="Note Basename">` inside
 *     the brief is intercepted in the child, posted up as `{__hc:1, kind:'navigate'}`,
 *     validated against this exact frame, and handed to `onNavigate`. The iframe
 *     can only ASK the trusted parent to navigate — it never navigates the app
 *     itself (null origin), so the sandbox guarantee is intact. Regular
 *     `<a href="https://…">` / `mailto:` links are posted up the same way
 *     (`{__hc:1, kind:'open'}`, canvas bridge v1) and opened by `openFrameLink`; the main window's
 *     navigation guard blocks the frame from following them itself.
 */

/** Presentational base for a document-style brief. The brief's own inline
 *  <style> comes later in the cascade and wins, so this is just a sane reset.
 *  The body padding is the document's ONE page gutter and it's FLUID (vw-based
 *  clamp): it grows and shrinks with the frame width — which tracks the pane,
 *  which tracks the rail + aside — instead of stranding a fixed margin. The
 *  doc's own outer wrapper padding is neutralized (FILL_STYLES) so this is the
 *  single source of the page margin and nothing stacks. */
const DOC_STYLES = `html,body{margin:0}
body{padding:clamp(18px,2.6vw,48px);color:var(--app-ink);background:transparent;font-family:system-ui,-apple-system,'Segoe UI',sans-serif;font-size:calc(15px * var(--app-text-scale,1));line-height:1.6;-webkit-font-smoothing:antialiased}
a{color:var(--app-primary)}
::selection{background:var(--app-primary);color:#fff}
img,svg,canvas,video,table{max-width:100%}
table{border-collapse:collapse}
th,td{border:1px solid var(--app-hairline);padding:6px 10px;text-align:left}
code,pre{font-family:var(--app-mono)}`;

/** Host-injected override that WINS the cascade (emitted after the document's
 *  own <style>). Workflow/vault HTML docs almost always wrap their content in a
 *  narrow centered column with its own page padding (e.g.
 *  `.wrap{max-width:720px;margin:0 auto;padding:32px 22px 48px}`), which both
 *  strands the document in the middle of a wide pane AND stacks its padding on
 *  top of the host pane's + the frame's, piling up dead space. We neutralize
 *  the wrapper's width cap, centering margin, and outer page padding so the
 *  document fills the width the pane gives it and the frame's single FLUID body
 *  gutter (DOC_STYLES) is the only page margin — nothing stacks. Scoped to
 *  DIRECT body children (plus a width-cap-only reset one level deeper for docs
 *  that nest their column wrapper), so the doc's own internal rhythm (section
 *  margins, nested card padding, `img,table{max-width:100%}` resets) is
 *  untouched; only the structural wrapper's outer box is reset.
 *
 *  The height/overflow resets are what keep the height ping honest: a doc that
 *  pins `html`/`body` (or its top wrapper) to `100vh` with an internal scroll
 *  region would otherwise report viewport-sized scrollHeight, leaving the
 *  frame short — the brief rendered cut off with a nested scrollbar and a
 *  blank band below. The document must flow naturally; the PANE scrolls. */
const FILL_STYLES =
  `html,body{height:auto!important;max-height:none!important;overflow:visible!important}` +
  `body>*{max-width:none!important;margin:0!important;padding:0!important;` +
  `height:auto!important;max-height:none!important;min-height:0!important;overflow:visible!important}` +
  `body>*>*{max-width:none!important}`;

/** Finite ceiling so a runaway/ResizeObserver-loop document can't set a
 *  multi-million-px height and lock up layout. */
const MAX_H = 20000;

/**
 * Normalize a brief into a body fragment for the sandbox scaffold. The workflow
 * writes a full self-contained HTML document; we strip the outer document tags
 * (keeping their inner content, including any <style>) so the brief renders
 * inside OUR <head> — which is what guarantees the CSP, theme vars, height ping
 * and nav bridge are present regardless of what the model emitted. A plain
 * fragment passes through unchanged.
 */
function toFragment(html: string): string {
  return html
    .replace(/<!doctype[^>]*>/gi, '')
    .replace(/<\/?html[^>]*>/gi, '')
    .replace(/<\/?head[^>]*>/gi, '')
    .replace(/<\/?body[^>]*>/gi, '')
    .replace(/<title[\s\S]*?<\/title>/gi, '')
    .replace(/<meta[^>]*>/gi, '')
    .trim();
}

/** The document as nonce-slotted parts (`sandboxDocParts`). The height ping /
 *  nav bridge / error bridge ride in via the host runtime. Artifact-authored
 *  scripts run only in a scripted policy (Canvas, Interactive). */
function buildDoc(code: string, mode: ArtifactMode, kit: CanvasKitAssets, look?: Look | null): string[] {
  return sandboxDocParts(DOC_STYLES, mode, kit, `\n${toFragment(code)}\n<style>${FILL_STYLES}</style>\n`, undefined, look);
}

export function HtmlDoc(props: {
  lookWorkspace?: string | null;
  code: string;
  onNavigate?: (target: string) => void;
  /** Runtime script errors from inside the document (chat preview uses this
   *  to surface broken artifacts and offer an agent fix). */
  onJsError?: (message: string) => void;
  /** The policy it opens in (10-11 B1): Canvas for an `.html` an agent
   *  wrote in Hollow (`fileTrust`), else Offline (HTML from outside). */
  defaultMode?: ArtifactMode;
  /** Where a permission the person gives is remembered: the file's path.
   *  Default: a hash of `code`. */
  grantKey?: string;
}) {
  // 10-11 B1: no policy buttons. The default, raised by a remembered grant;
  // a refused load raises one permission line.
  const base = () => props.defaultMode ?? DEFAULT_ARTIFACT_MODE;
  const grantKey = () => props.grantKey ?? textKey(props.code);
  const [mode, setMode] = createSignal<ArtifactMode>(grantedMode(grantKey(), base(), props.code));
  const [asked, setAsked] = createSignal<{ want: BlockedWant; host: string } | null>(null);
  const [notNow, setNotNow] = createSignal(false);
  // The preview pane keeps this mounted while its file changes (a turn's end,
  // the picker, a file link): another file, or new content, starts from its
  // own default and grant, never the previous file's (review B1 high).
  createEffect(on(() => [grantKey(), base(), props.code] as const, ([k, b, c]) => batch(() => {
    setMode(grantedMode(k, b, c)); setAsked(null); setNotNow(false);
  }), { defer: true }));
  function grant(next: ArtifactMode | null) {
    setGrant(grantKey(), next, props.code);
    batch(() => { setAsked(null); setMode(next ?? base()); });
  }
  const [reportedH, setReportedH] = createSignal(420);
  let frame: HTMLIFrameElement | undefined;
  let frameScripted = false;

  // The Canvas Kit (inlined into every document) loads once, in its own
  // chunk. A signal, not a resource, so the first load never suspends a page.
  const [kit, setKit] = createSignal<CanvasKitAssets | null>(loadedCanvasKit());
  if (!kit()) void loadCanvasKit().then(setKit);
  const look = () => lookFor(props.lookWorkspace);
  const parts = createMemo(() => { const k = kit(); return k ? buildDoc(props.code, mode(), k, untrack(look)) : null; });
  // Outside Tauri (no artifact:// URL) the document falls back to srcdoc, which
  // inherits the app CSP: Canvas renders Offline there rather than pretending.
  // The fallback seals the parts with a nonce of its own.
  const fallbackSrcdoc = createMemo(() => {
    const k = kit(), p = parts();
    if (!k || !p) return '';
    return sealDoc(srcdocMode(mode()) === mode() ? p : buildDoc(props.code, srcdocMode(mode()), k, untrack(look)));
  });
  const fallbackNote = () => !docUrl.loading && !docUrl() && srcdocMode(mode()) !== mode();
  const raised = () => mode() !== base();
  // Preferred load path: register the document with the backend and load it
  // off the artifact:// protocol, where the response carries the selected
  // CSP. Falls back to srcdoc outside Tauri. Keyed on the built document so
  // an edited artifact re-registers; identical content re-renders reuse the
  // same URL.
  const [docUrl] = createResource(
    () => { const p = parts(); return p ? { parts: p, mode: mode() } : false; },
    ({ parts, mode }) => artifactDocUrl(parts, mode),
  );

  function onMsg(e: MessageEvent) {
    const r = acceptFrameMessage(e, frame);
    // A refused message from this frame is logged (C6 P1), never its payload.
    if (!r.ok && r.reason !== 'source' && r.reason !== 'origin') logBridgeRefusal(e.data, r.reason, null, null);
    if (!r.ok || !frame) return;
    const m = r.message;
    if (m.kind === 'size') setReportedH(Math.max(80, Math.min(m.h, MAX_H)));
    else if (m.kind === 'navigate') props.onNavigate?.(m.target);
    else if (m.kind === 'error') props.onJsError?.(m.message);
    else if (m.kind === 'open') openFrameLink(m, frame, frameScripted);
    else if (m.kind === 'blocked' && !asked() && !notNow() && !allows(mode(), m.want)) setAsked({ want: m.want, host: m.host });
  }
  let stopTheme: (() => void) | undefined;
  onMount(() => window.addEventListener('message', onMsg));
  onCleanup(() => { window.removeEventListener('message', onMsg); stopTheme?.(); });

  return (
    <div class="html-doc" data-sandbox={mode()}>
      {/* 10-11 B1: a line only when something was refused, or while a
          permission the person gave is on (so it can be turned off). */}
      <Show when={asked() || raised() || fallbackNote()}>
        <div class="artifact-ask html-doc-ask" role="status" data-want={asked()?.want ?? (raised() ? 'granted' : 'fallback')}>
          <Show when={asked()} fallback={<Show when={raised()} fallback={<span class="artifact-ask-text">Scripts run only in the Hollow app; showing this without them.</span>}>
            <span class="artifact-ask-text">{mode() === 'canvas' ? 'Scripts are on for this file.' : 'This file may reach the internet.'}</span>
            <button type="button" class="btn small ghost" onClick={() => { const off = withoutInternet(mode()); grant(mode() === 'canvas' || off === base() ? null : off); }}>{mode() === 'canvas' ? 'Turn scripts off' : 'Turn internet off'}</button>
          </Show>}>
            {(a) => <>
              <span class="artifact-ask-text">{blockedLine(a().want, a().host, 'file')}</span>
              <button type="button" class="btn small" onClick={() => grant(escalate(mode(), a().want))}>{a().want === 'scripts' ? 'Run scripts' : 'Allow'}</button>
              <button type="button" class="btn small ghost" onClick={() => batch(() => { setAsked(null); setNotNow(true); })}>Not now</button>
            </>}
          </Show>
        </div>
      </Show>
      {/* Hold the frame until the artifact URL settles (a fast local IPC)
          so the document loads exactly once — srcdoc is only the non-Tauri
          fallback, where artifact JS wasn't running anyway. */}
      <Show when={parts() && !docUrl.loading}>
        <iframe
          ref={el => { frame = el; frameScripted = modeRunsScripts(mode()); stopTheme?.(); stopTheme = watchFrameTheme(el, look); }}
          class="html-doc-frame"
          sandbox={SANDBOX}
          src={docUrl() ?? undefined}
          srcdoc={docUrl() ? undefined : fallbackSrcdoc()}
          style={{ height: `${reportedH()}px` }}
          title="Rendered brief"
        />
      </Show>
    </div>
  );
}
