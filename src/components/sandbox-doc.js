/**
 * Host-side runtime for agent-authored HTML rendered in Hollow's sandboxed
 * iframes (HtmlArtifact chat fences, HtmlDoc workflow briefs / preview pane).
 * Embedded VERBATIM into the srcdoc scaffold (imported `?raw` by
 * htmlSandbox.ts) and allowed by a sha256 hash in the app CSP.
 *
 * WHY A SEPARATE FILE + CSP HASH — load-bearing, learned the hard way:
 * a srcdoc document INHERITS the embedding page's Content-Security-Policy, and
 * the app's production CSP (tauri.conf.json) deliberately has no
 * 'unsafe-inline' in script-src. A plain inline script inside the sandbox
 * therefore runs in `tauri dev` (no devCsp is set → no CSP) but is silently
 * BLOCKED in installed builds — which froze every brief at the host's
 * placeholder height (the "half-blank pane / clipped brief" bug). Loading it
 * as an external `script-src 'self'` asset doesn't work either: the inherited
 * policy is evaluated in the CHILD document, whose sandboxed origin is
 * opaque, so 'self' matches nothing there (verified: NO PINGS in a release
 * build). A hash-source matches the script's CONTENT, independent of origin —
 * so the exact sha256 of this file's bytes is listed in tauri.conf.json's
 * script-src. `scripts/sync-sandbox-csp.mjs` (run by `npm run build`)
 * recomputes and rewrites that hash, so editing this file can never silently
 * break installed builds.
 *
 * Six jobs, all over postMessage (the frame is null-origin). Every message is
 * a canvas bridge v1 envelope, `{__hc:1, kind, …}`, typed and validated on the
 * host by `src/canvasBridge.ts` (sender frame, 64 KB cap, exact fields).
 *
 *  1. HEIGHT PING — `{__hc:1, kind:'size', h}` so the host can size the frame
 *     to content (the pane scrolls, never the frame). Two subtleties found the
 *     hard way:
 *       - WKWebView's ResizeObserver fires once in sandboxed srcdoc frames and
 *         then goes dead, so host resizes never re-measured. The inner
 *         window's `resize` event DOES fire reliably; a trailing re-measure
 *         catches late reflow.
 *       - documentElement.scrollHeight is floored at the viewport height, so
 *         counting it unconditionally ratchets the frame tall (it can never
 *         report smaller than the current frame). Only count it when it
 *         exceeds innerHeight; the html/body element boxes track content.
 *
 *  2. NAV BRIDGE — capture-phase click interception of `a[data-vault]` →
 *     `{__hc:1, kind:'navigate', target}` (fires before
 *     `<base target="_blank">` would follow the link). The frame can only ASK
 *     the trusted parent to navigate; hosts that don't care ignore it. Web
 *     and mail links (`a[href]` starting http(s):// or mailto:) →
 *     `{__hc:1, kind:'open', href}`: the main window's navigation guard
 *     (main_nav.rs) blocks the frame from loading them, and the host opens
 *     them (store/browser.ts openFrameLink, confirmed for scripted modes).
 *
 *  3. ERROR BRIDGE — script errors / unhandled rejections →
 *     `{__hc:1, kind:'error', message}` so the chat preview pane can
 *     surface broken artifacts and offer an agent fix.
 *
 *  4. THEME (host → child) — `{__hc:1, kind:'theme', scheme, vars}` from the
 *     parent window only: sets each `--app-*` token and `color-scheme` on
 *     <html>, so a theme switch restyles a mounted frame without a reload,
 *     then fires a `hollow-theme` event for canvas code that draws with them.
 *     Names and values are re-checked here with the host's rules.
 *
 *  5. INIT AND STATE (Canvas C3) — the host posts `{__hc:1, kind:'init',
 *     canvasId, kit, locale, reducedMotion, state}` once per load (the first
 *     one wins). The runtime keeps the saved state as a key → JSON map and
 *     exposes it through a read-only global, `hollowCanvas`: `init` (null
 *     until init arrives, then a frozen {canvasId, kit, locale,
 *     reducedMotion}), `get(key)` (a copy) and `set(key, value)` (undefined
 *     removes; false when this frame is not a canvas or the map would pass
 *     16 KB), and fires `hollow-init` with the same fields plus `state`. A
 *     `set` posts `{__hc:1, kind:'state', canvasId, state}` (the whole map)
 *     after a short debounce; the host saves it only for this frame's canvas
 *     and only while the person is using the frame. Reduced motion also sets
 *     `data-hc-motion="reduce"` on <html> for the kit's stylesheet.
 *
 *  6. REFUSED LOADS AND VIEW SOURCE (10-11 B1) — the first CSP refusal of
 *     each kind → `{__hc:1, kind:'blocked', want, host}` (`images`,
 *     `network` or `scripts`; the host name only), which the host shows as
 *     one permission line; ⌥⌘U inside the frame → `{__hc:1, kind:'source'}`.
 *
 *  7. DESIGN CHECK (repo ideas 2026-10-27 #1) — only when the host asks,
 *     `{__hc:1, kind:'design-check', ticket}` (reply canvases), and only
 *     once this load has settled: one bounded pass over the agent's own
 *     text elements (never the kit's: hc-* elements and their rendered
 *     parts) → `{__hc:1, kind:'design', ticket, items, scanned, more,
 *     complete, width, texture}` (`complete`: every text element was looked
 *     at; `width` and `texture`: how this frame drew, so the host can draw
 *     it again to measure an unresolved text's box on the picture).
 *
 * The host window is captured before any artifact script runs: a script can
 * shadow the global `parent`, but not this closure's copy.
 */
(function () {
  'use strict';

  var host = window.parent;
  function post(kind, fields) {
    var message = { __hc: 1, kind: kind };
    for (var k in fields) message[k] = fields[k];
    try { host.postMessage(message, '*'); } catch (e) {}
  }

  // CSP connect-src does not confine WebRTC's STUN/TURN traffic. C0's
  // native loopback listener observed UDP even with connect-src https:.
  // This host runtime precedes artifact scripts. Remove both constructors
  // irreversibly in this opaque frame before any untrusted code executes.
  ['RTCPeerConnection', 'webkitRTCPeerConnection'].forEach(function (name) {
    Object.defineProperty(window, name, { value: undefined, writable: false, configurable: false });
  });

  /* ── 1. Height ping ────────────────────────────────────────────────── */

  function measure() {
    var d = document.documentElement;
    var b = document.body;
    var h = Math.max(d.offsetHeight, b ? b.scrollHeight : 0, b ? b.offsetHeight : 0);
    // Viewport-floored, so only informative when content overflows the html box.
    if (d.scrollHeight > innerHeight) h = Math.max(h, d.scrollHeight);
    return Math.ceil(h);
  }

  function report() {
    post('size', { h: measure() });
  }

  var resizeTimer;
  addEventListener('resize', function () {
    report();
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(report, 150);
  });

  function observe() {
    try { new ResizeObserver(report).observe(document.documentElement); } catch (e) {}
    try { if (document.body) new ResizeObserver(report).observe(document.body); } catch (e) {}
    report();
  }
  // This script loads from <head>, before <body> exists — defer the observers.
  if (document.readyState === 'loading') addEventListener('DOMContentLoaded', observe);
  else observe();

  addEventListener('load', report);
  setTimeout(report, 60);
  setTimeout(report, 300);
  setTimeout(report, 1200); // late layout: web fonts, images

  /* ── 2. Vault-link and web-link navigation bridge ──────────────────── */

  addEventListener('click', function (e) {
    var t = e.target;
    var a = t && t.closest ? t.closest('a[data-vault]') : null;
    if (a) {
      e.preventDefault();
      post('navigate', { target: String(a.getAttribute('data-vault') || '') });
      return;
    }
    var w = t && t.closest ? t.closest('a[href]') : null;
    var href = w ? String(w.getAttribute('href') || '').trim() : '';
    if (!/^(https?:\/\/|mailto:)/i.test(href)) return;
    e.preventDefault();
    post('open', { href: href });
  }, true);

  /* ── 3. Runtime-error bridge ───────────────────────────────────────── */

  function reportError(message) {
    post('error', { message: String(message).slice(0, 500) });
  }
  addEventListener('error', function (e) {
    reportError(e.message + (e.lineno ? ' (line ' + e.lineno + ')' : ''));
  });
  addEventListener('unhandledrejection', function (e) {
    reportError('Unhandled rejection: ' + ((e.reason && e.reason.message) || e.reason));
  });

  /* ── 4. Theme updates from the host ────────────────────────────────── */

  var THEME_VAR = /^--app-[a-z0-9-]{1,40}$/;
  var UNSAFE_VALUE = /[;{}<>\\\u0000-\u001f\u007f]/;
  addEventListener('message', function (e) {
    if (e.source !== host) return;
    var d = e.data;
    if (!d || d.__hc !== 1 || d.kind !== 'theme' || !d.vars || typeof d.vars !== 'object') return;
    var names = Object.keys(d.vars);
    if (names.length > 48) return;
    var root = document.documentElement.style;
    for (var i = 0; i < names.length; i++) {
      var name = names[i];
      var value = d.vars[name];
      if (THEME_VAR.test(name) && typeof value === 'string' && value.length > 0 && value.length <= 400 && !UNSAFE_VALUE.test(value)) {
        root.setProperty(name, value);
      }
    }
    if (d.scheme === 'light' || d.scheme === 'dark') root.setProperty('color-scheme', d.scheme);
    try { dispatchEvent(new CustomEvent('hollow-theme', { detail: { scheme: d.scheme } })); } catch (err) {}
  }, true);

  /* ── 5. Init and saved state ───────────────────────────────────────── */

  var CANVAS_ID = /^[a-z0-9-]{1,64}$/;
  var info = null;
  var state = Object.create(null);
  var stateTimer;

  function copy(value) {
    return value === undefined ? undefined : JSON.parse(JSON.stringify(value));
  }
  function bytes(value) {
    try { return new TextEncoder().encode(JSON.stringify(value)).length; } catch (e) { return Infinity; }
  }
  // A null-prototype map, so a "__proto__" key is data, never a prototype.
  function toMap(source) {
    var map = Object.create(null);
    if (source && typeof source === 'object' && !Array.isArray(source)) {
      for (var k in source) if (Object.prototype.hasOwnProperty.call(source, k)) map[k] = source[k];
    }
    return map;
  }

  var textureAllowed = false;
  var textureLevel = 'full';
  Object.defineProperty(window, 'hollowCanvas', {
    value: Object.freeze({
      get init() { return info; },
      get textureAllowed() { return textureAllowed; },
      get textureLevel() { return textureLevel; },
      get: function (key) { return copy(state[String(key)]); },
      set: function (key, value) {
        if (!info || !info.canvasId) return false;
        var next = toMap(state);
        key = String(key);
        if (value === undefined) delete next[key];
        else {
          try { next[key] = copy(value); } catch (e) { return false; }
        }
        if (bytes(next) > 16384) return false;
        state = next;
        clearTimeout(stateTimer);
        stateTimer = setTimeout(function () {
          post('state', { canvasId: info.canvasId, state: copy(state) });
        }, 120);
        return true;
      },
    }),
    writable: false,
    configurable: false,
  });

  addEventListener('message', function (e) {
    if (e.source !== host) return;
    var d = e.data;
    if (!d || d.__hc !== 1 || d.kind !== 'init') return;
    if (info) return;
    var canvasId = typeof d.canvasId === 'string' && CANVAS_ID.test(d.canvasId) ? d.canvasId : null;
    var saved = canvasId && d.state && typeof d.state === 'object' && !Array.isArray(d.state) && bytes(d.state) <= 16384 ? copy(d.state) : null;
    info = Object.freeze({
      canvasId: canvasId,
      kit: typeof d.kit === 'string' ? d.kit.slice(0, 32) : '',
      locale: typeof d.locale === 'string' && /^[A-Za-z0-9-]{2,35}$/.test(d.locale) ? d.locale : 'en',
      reducedMotion: d.reducedMotion === true,
    });
    state = toMap(saved);
    if (info.reducedMotion) document.documentElement.setAttribute('data-hc-motion', 'reduce');
    var detail = { canvasId: info.canvasId, kit: info.kit, locale: info.locale, reducedMotion: info.reducedMotion, state: copy(saved) };
    try { dispatchEvent(new CustomEvent('hollow-init', { detail: detail })); } catch (err) {}
    var t, done=false, fonts=false;
    function end(){if(done)return;done=true;clearTimeout(t);o.disconnect();requestAnimationFrame(function(){requestAnimationFrame(function(){post('settled',{h:measure()});settledOnce=true;if(designTicket)runDesign();});});}
    function quiet(){clearTimeout(t);if(fonts&&!done)t=setTimeout(end,250);}
    var o=new MutationObserver(quiet);o.observe(document,{subtree:true,childList:true,attributes:true,characterData:true});
    Promise.resolve(document.fonts&&document.fonts.ready).then(function(){fonts=true;quiet();});setTimeout(end,4000);
  }, true);

  // Live app motion preference; OS Reduce Motion is also checked in the kit.
  // Only the parent can set it, like theme/init. No write goes back.
  addEventListener('message', function (e) {
    var d = e.data;
    if (e.source !== host || !d || d.__hc !== 1 || d.kind !== 'motion' || typeof d.reducedMotion !== 'boolean') return;
    if (d.reducedMotion) document.documentElement.setAttribute('data-hc-motion', 'reduce');
    else document.documentElement.removeAttribute('data-hc-motion');
    var nextTexture = !d.reducedMotion && d.textureAllowed === true;
    var nextLevel = d.textureLevel === 'quiet' ? 'quiet' : 'full';
    if (textureAllowed !== nextTexture || textureLevel !== nextLevel) {
      textureAllowed = nextTexture;
      textureLevel = nextLevel;
      try { dispatchEvent(new Event('hollow-texture')); } catch (err) {}
    }
  }, true);

  // Pinned live canvases: scalar host data only. No command, query, path or
  // selector comes from the frame. Updating values keeps the iframe/UI state.
  addEventListener('message', function (e) {
    var d = e.data;
    if (e.source !== host || !d || d.__hc !== 1 || d.kind !== 'live-data' || !d.values || typeof d.values !== 'object' || bytes(d.values) > 4096) return;
    var keys = ['plan.done', 'plan.open', 'plan.total', 'tasks.open', 'tasks.done', 'tasks.total', 'usage.runs', 'usage.reportedUsd', 'sessions.running'];
    document.querySelectorAll('[data-hollow-bind]').forEach(function (el) {
      var key = el.getAttribute('data-hollow-bind');
      if (keys.indexOf(key) < 0 || !Object.prototype.hasOwnProperty.call(d.values, key)) return;
      var value = d.values[key];
      if (value !== null && (typeof value !== 'number' || !Number.isFinite(value))) return;
      if (el.tagName === 'SPAN') el.textContent = value === null ? 'Unavailable' : String(value);
      else if (el.tagName === 'HC-STAT') el.setAttribute('value', value === null ? 'Unavailable' : String(value));
      else if (el.tagName === 'HC-GAUGE' || el.tagName === 'HC-PROGRESS') {
        if (value === null) el.removeAttribute('value'); else el.setAttribute('value', String(value));
      }
    });
  }, true);

  /* ── 6. Refused loads and the view-source key (10-11 B1) ───────────── */

  // The browser refuses what this document's policy doesn't allow and fires
  // `securitypolicyviolation`; the host turns the first refusal of each kind
  // into one permission line ("tried to load images from x.com"). Only the
  // host name travels, never the path or query. Inline-script refusals
  // (a locked file's scripts) report `scripts` with no host.
  // A remote script needs both: scripts first, then the network (the host
  // asks for one at a time; a locked page that is granted scripts reloads,
  // is refused again, and then asks for the network).
  var refused = Object.create(null);
  // Captured before any canvas script can replace it.
  var HostURL = typeof URL === 'function' ? URL : null;
  function refusal(w, uri) {
    if (refused[w]) return;
    var name = '';
    if (w !== 'scripts') {
      if (!HostURL) return;
      try { name = new HostURL(uri).hostname.toLowerCase().replace(/\.$/, ''); } catch (err) { return; }
      if (!name || name.length > 253) return;
      // A relative URL resolves against this document's own base (the app
      // page, in the srcdoc fallback): not a remote load to ask about.
      try { if (new HostURL(uri).origin === new HostURL(document.baseURI).origin) return; } catch (err) {}
    }
    refused[w] = true;
    post('blocked', { want: w, host: name });
  }
  addEventListener('securitypolicyviolation', function (e) {
    var d = String(e.effectiveDirective || e.violatedDirective || '');
    var uri = String(e.blockedURI || '');
    var remote = /^https?:/i.test(uri);
    if (/^script-src/.test(d)) {
      // WebKit may report a refused inline script with an empty URI.
      if (uri === 'inline' || uri === '' || remote) refusal('scripts', uri);
      if (remote) refusal('network', uri);
    } else if (remote && d === 'img-src') refusal('images', uri);
    else if (remote && /^(connect|font|media|style)-src/.test(d)) refusal('network', uri);
  }, true);
  // ⌥⌘U reaches the host from inside the frame (keys typed here never do).
  addEventListener('keydown', function (e) {
    if (e.metaKey && e.altKey && !e.ctrlKey && !e.shiftKey && (e.code === 'KeyU' || String(e.key).toLowerCase() === 'u')) {
      e.preventDefault();
      post('source', {});
    }
  }, true);

  /* ── 7. Design check (repo ideas 2026-10-27 #1) ────────────────────── */

  // One pass per load, after this frame settled, and only when the host
  // asked (`design-check`). It reads computed styles and changes nothing.
  // Kit elements and their rendered parts are the kit's: skipped. Text over
  // a gradient, image, canvas or texture is reported as unresolved, never
  // guessed; unpainted and mid-animation elements are skipped. Bounded:
  // 5000 elements walked, 1500 text elements measured, 24 items sent.
  var settledOnce = false, designTicket = null, designDone = false;
  var colours = Object.create(null), colourCtx = null;
  // Any CSS colour as [r, g, b, a] (0–255, alpha 0–1), through a 1×1 canvas
  // so oklch(), color-mix() and named colours resolve too; null if unknown.
  function rgba(css) {
    if (css in colours) return colours[css];
    var out = null;
    var m = /^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)\s*(?:[,/]\s*([\d.]+)(%?)\s*)?\)$/.exec(css);
    if (m) out = [+m[1], +m[2], +m[3], m[4] === undefined ? 1 : m[5] ? +m[4] / 100 : +m[4]];
    else {
      try {
        if (!colourCtx) { var c = document.createElement('canvas'); c.width = c.height = 1; colourCtx = c.getContext('2d', { willReadFrequently: true }); }
        var probe = function (start) {
          colourCtx.clearRect(0, 0, 1, 1); colourCtx.fillStyle = start; colourCtx.fillStyle = css; colourCtx.fillRect(0, 0, 1, 1);
          var d = colourCtx.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3] / 255];
        };
        var a = colourCtx && probe('#000'), b = colourCtx && probe('#fff');
        if (a && b && a.join() === b.join()) out = a;
      } catch (err) {}
    }
    colours[css] = out;
    return out;
  }
  // `top` over `below`, both [r, g, b, a] with any alpha (normal blending).
  function over(top, below) {
    var a = top[3] + below[3] * (1 - top[3]);
    if (a <= 0) return [0, 0, 0, 0];
    var mix = function (i) { return (top[i] * top[3] + below[i] * below[3] * (1 - top[3])) / a; };
    return [mix(0), mix(1), mix(2), a];
  }
  function luminance(c) {
    var v = [c[0], c[1], c[2]].map(function (x) { x /= 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); });
    return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
  }
  function ownText(el) {
    var s = '';
    for (var n = el.firstChild; n; n = n.nextSibling) if (n.nodeType === 3) s += n.nodeValue;
    return s.replace(/[\u0000-\u001f\u007f]+/g, ' ').replace(/\s+/g, ' ').trim();
  }
  // The box of an element's own text lines in whole document CSS px, inside
  // the document, or null when the picture can't measure it alone: another
  // element's text or a form control's value would be measured with it; its
  // glyphs paint a clipped background (`background-clip: text` in any layer),
  // which a transparent fill doesn't remove; the canvas has animation of its
  // own in effect (the capture freezes animation, which may move things); or
  // the lines lie outside the document or past 100000.
  var CLIP_TEXT = /(^|,)\s*text\s*(,|$)/;
  function ownTextBox(el, animated) {
    if (animated) return null;
    for (var c = el.firstElementChild; c; c = c.nextElementSibling) if ((c.textContent || '').trim()) return null;
    if (el.querySelector('input,textarea,select,button,option')) return null;
    for (var a = el; a && a.nodeType === 1; a = a.parentElement) {
      var as = getComputedStyle(a);
      if (CLIP_TEXT.test(as.backgroundClip || '') || CLIP_TEXT.test(as.webkitBackgroundClip || '')) return null;
    }
    var l = Infinity, t = Infinity, rr = -Infinity, b = -Infinity;
    for (var n = el.firstChild; n; n = n.nextSibling) {
      if (n.nodeType !== 3 || !n.nodeValue.trim()) continue;
      var range = document.createRange();
      range.selectNodeContents(n);
      var rects = range.getClientRects();
      for (var k = 0; k < rects.length; k++) {
        var q = rects[k];
        if (q.width < 1 || q.height < 1) continue;
        l = Math.min(l, q.left); t = Math.min(t, q.top); rr = Math.max(rr, q.right); b = Math.max(b, q.bottom);
      }
    }
    var root = document.documentElement;
    var x0 = Math.max(0, Math.floor(l + scrollX)), y0 = Math.max(0, Math.floor(t + scrollY));
    var x1 = Math.min(root.scrollWidth, Math.ceil(rr + scrollX)), y1 = Math.min(root.scrollHeight, Math.ceil(b + scrollY));
    if (!(x1 - x0 >= 1 && y1 - y0 >= 1) || x1 > 100000 || y1 > 100000) return null;
    return [x0, y0, x1 - x0, y1 - y0];
  }
  function kitOwned(el) {
    return /^HC-/.test(el.tagName) || !!(el.closest && el.closest('[data-hc-part]'));
  }
  function runDesign() {
    if (designDone || !designTicket) return;
    designDone = true;
    // `complete`: every text element was looked at (no cap reached, none
    // skipped mid-animation, nothing thrown). Only then may the host let the
    // rendered sizes overrule the source's.
    var items = [], more = 0, scanned = 0, complete = false, skipped = 0;
    function add(item) { if (items.length < 24) items.push(item); else more++; }
    try {
      // `ownMotion`: an animation other than the kit's own (`id` "hc-kit":
      // transform only, inside its elements) is in effect, running or
      // finished and filling forwards, on any element; a capture with
      // animation frozen may lay out differently, so no unresolved text gets
      // a box to measure.
      var moving = [], ownMotion = false;
      try {
        if (document.getAnimations) document.getAnimations().forEach(function (a) {
          var target = a.effect && a.effect.target;
          if (!target) return;
          if (a.playState === 'running') moving.push(target);
          if (a.id !== 'hc-kit') ownMotion = true;
        });
      } catch (err) {}
      var groundCss = '';
      try { groundCss = getComputedStyle(document.documentElement).getPropertyValue('--app-canvas').trim(); } catch (err) {}
      var base = (groundCss && rgba(groundCss)) || [255, 255, 255, 1];
      if (base[3] < 1) base = over(base, [255, 255, 255, 1]);
      var cache = new Map();
      // What an element's own box puts under text: a colour, or a texture.
      var facts = function (el) {
        var f = cache.get(el);
        if (f) return f;
        var cs = getComputedStyle(el);
        var image = cs.backgroundImage !== 'none' || cs.backgroundClip === 'text' || cs.webkitBackgroundClip === 'text';
        ['::before', '::after'].forEach(function (p) {
          if (image) return;
          var ps = getComputedStyle(el, p);
          if (ps.content !== 'none' && ps.backgroundImage !== 'none' && (ps.position === 'absolute' || ps.position === 'fixed')) image = true;
        });
        f = { bg: rgba(cs.backgroundColor), image: image, opacity: parseFloat(cs.opacity) };
        if (!(f.opacity >= 0)) f.opacity = 1;
        cache.set(el, f);
        return f;
      };
      // Positioned pictures and patterned boxes that may sit over or under text.
      var all = document.body ? document.body.getElementsByTagName('*') : [];
      var decor = [];
      for (var i = 0; i < all.length && i < 5000 && decor.length < 200; i++) {
        var d = all[i], tag = d.tagName.toLowerCase();
        if (d.closest && d.closest('svg') && tag !== 'svg') continue;
        var ds = getComputedStyle(d);
        if (ds.position !== 'absolute' && ds.position !== 'fixed') continue;
        if (tag === 'img' || tag === 'canvas' || tag === 'video' || tag === 'svg' || ds.backgroundImage !== 'none') {
          var dr = d.getBoundingClientRect();
          if (dr.width >= 1 && dr.height >= 1) decor.push({ el: d, r: dr });
        }
      }
      for (var j = 0; j < all.length && j < 5000 && scanned < 1500; j++) {
        var el = all[j];
        if (/^(SCRIPT|STYLE|TEMPLATE|NOSCRIPT|TITLE)$/.test(el.tagName) || el.namespaceURI !== 'http://www.w3.org/1999/xhtml') continue;
        var text = ownText(el);
        if (!text || kitOwned(el)) continue;
        var r = el.getBoundingClientRect();
        var cs = getComputedStyle(el);
        // Unpainted, hidden, or a 1px screen-reader-only box.
        if (r.width <= 1 || r.height <= 1 || cs.visibility !== 'visible') continue;
        var animated = false;
        for (var k = 0; k < moving.length && !animated; k++) if (moving[k] === el || (moving[k].contains && moving[k].contains(el))) animated = true;
        if (animated) { skipped++; continue; }
        scanned++;
        var sample = text.slice(0, 80);
        var size = parseFloat(cs.fontSize) || 0;
        if (size > 0 && size < 11) add({ rule: 'small-text', text: sample, size: Math.round(size * 100) / 100 });
        var fg = rgba(cs.color);
        if (!fg) continue;
        // A glyph pixel and the ground beside it, composited outwards through
        // every box around the text: each box's background goes under what
        // is inside it, then its opacity applies to the whole group, as the
        // page paints it. A background image (or pattern) still showing
        // through makes the ground unknown; the host's canvas colour is last.
        var ink = [fg[0], fg[1], fg[2], fg[3]], ground = [0, 0, 0, 0], textured = false, alpha = 1;
        for (var n = el; n && n.nodeType === 1; n = n.parentElement) {
          var f = facts(n);
          if (f.image && ground[3] < 1) textured = true;
          if (f.bg && f.bg[3] > 0) { ink = over(ink, f.bg); ground = over(ground, f.bg); }
          ink[3] *= f.opacity; ground[3] *= f.opacity; alpha *= f.opacity;
        }
        ink = over(ink, base); ground = over(ground, base);
        if (!textured) {
          for (var q = 0; q < decor.length && !textured; q++) {
            var de = decor[q];
            if (de.el === el || de.el.contains(el) || el.contains(de.el)) continue;
            var w = Math.min(r.right, de.r.right) - Math.max(r.left, de.r.left);
            var h = Math.min(r.bottom, de.r.bottom) - Math.max(r.top, de.r.top);
            if (w > 0 && h > 0 && w * h > 0.3 * r.width * r.height) textured = true;
          }
        }
        if (alpha < 0.1) { scanned--; continue; }
        var weight = parseInt(cs.fontWeight, 10) || 400;
        var need = size >= 24 || (size >= 18.66 && weight >= 700) ? 3 : 4.5;
        // Unresolved here, so the host may measure it on the picture (ideas
        // 10-27 #4): the box of its own text lines, or null.
        if (textured || fg[3] < 0.05) { add({ rule: 'texture-text', text: sample, need: need, box: ownTextBox(el, ownMotion) }); continue; }
        var l1 = luminance(ink), l2 = luminance(ground);
        var ratio = (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
        if (ratio < need) add({ rule: 'contrast', text: sample, ratio: Math.max(1, Math.round(ratio * 100) / 100), need: need });
      }
      complete = j >= all.length && !skipped;
    } catch (err) { complete = false; }
    var width = Math.max(1, Math.min(8000, Math.round(document.documentElement.clientWidth || innerWidth || 1)));
    post('design', { ticket: designTicket, items: items, scanned: scanned, more: Math.min(more, 100000), complete: complete, width: width, texture: textureAllowed === true });
  }
  addEventListener('message', function (e) {
    var d = e.data;
    if (e.source !== host || !d || d.__hc !== 1 || d.kind !== 'design-check' || typeof d.ticket !== 'string' || !/^[a-z0-9]{16,40}$/.test(d.ticket)) return;
    if (designTicket) return;
    designTicket = d.ticket;
    if (settledOnce) runDesign();
  }, true);
})();
