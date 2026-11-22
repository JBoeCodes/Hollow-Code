/** Shared artifact presentation; no app store, so capture stays storeless. */
export const ARTIFACT_STYLES = `html,body{margin:0}
body{padding:14px 16px;color:var(--app-ink);background:var(--app-canvas);font-family:system-ui,-apple-system,'Segoe UI',sans-serif;font-size:calc(14px * var(--app-text-scale,1));line-height:1.55;-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility}
:where(h1,h2,h3,h4){line-height:1.25;letter-spacing:-.01em;margin:1.1em 0 .45em}
:where(h1){font-size:1.45em;letter-spacing:-.02em}:where(h2){font-size:1.2em}:where(h3){font-size:1.05em}:where(h4){font-size:1em}
:where(body>:first-child,body>[data-hc-texture-root]>:first-child){margin-top:0}:where(body>:last-child,body>[data-hc-texture-root]>:last-child){margin-bottom:0}
:where(p){margin:.5em 0}:where(ul,ol){padding-left:1.3em}
a{color:var(--app-primary)}
::selection{background:var(--app-primary);color:#fff}
img,svg,canvas,video,table{max-width:100%}
table{border-collapse:collapse}
th,td{border:1px solid var(--app-hairline);padding:4px 8px;text-align:left}
code,pre{font-family:var(--app-mono)}`;
