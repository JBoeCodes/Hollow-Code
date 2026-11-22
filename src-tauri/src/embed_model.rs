//! candle runtime for the local semantic-search embedder.
//!
//! Pure-Rust (candle) on purpose: inference compiles INTO the signed Hollow
//! binary, exactly the way `rusqlite { bundled }` ships SQLite's C — there is
//! no native ONNX dylib to vendor, rpath, or re-sign (the rationale lives in
//! `docs/history/SEMANTIC_SEARCH_PLAN.md`). Model: BAAI/bge-small-en-v1.5 (384-dim,
//! **CLS pooling** then L2-normalize). The weights ship as a bundled Tauri
//! resource under `Contents/Resources/models/` (data, not code → no signing
//! implication).
//!
//! Phase 1 wires this into `vault_embed_index` (embed each chunk OFF the write
//! path). This module is standalone + lazy: the ~133 MB model loads on actual
//! work, is reused while active, and is released when disabled or idle. Runtime
//! failures are values, never panics: `[profile.release] panic = "abort"` means a panic on the
//! background embed thread would abort the whole app, so failures are values.

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use once_cell::sync::Lazy;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, TryLockError};
use std::time::{Duration, Instant};
use tokenizers::Tokenizer;

/// bge-small-en-v1.5 position-embedding limit (`max_position_embeddings`).
/// Inputs are truncated to this to avoid an index-out-of-bounds panic on an
/// unusually dense chunk.
const MAX_TOKENS: usize = 512;

/// bge retrieval instruction — prepended to QUERIES ONLY (never passages), so
/// the index side and the query side stay consistent.
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

/// Measured from the end of the most recent embedding. The semantic worker
/// checks this on its existing 30-second tick; an idle check never loads it.
pub const MODEL_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Only a failure while embedding a particular input may tombstone that chunk.
/// Cancellation and model-load failures must leave chunks pending for retry.
#[derive(Debug, PartialEq, Eq)]
pub enum EmbedError {
    Cancelled,
    Load(String),
    Inference(String),
}

impl fmt::Display for EmbedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("semantic embedding cancelled"),
            Self::Load(message) | Self::Inference(message) => f.write_str(message),
        }
    }
}

struct CachedModel<M> {
    model: Option<M>,
    last_used: Option<Instant>,
    epoch: u64,
}

impl<M> CachedModel<M> {
    fn clear(&mut self) -> bool {
        self.last_used = None;
        self.model.take().is_some()
    }
}

/// The lifecycle gate covers only unload bookkeeping and the final cache
/// unlock, never loading or inference. This makes disable nonblocking during
/// inference without a missed-unload race at the end of an active operation.
struct ModelCache<M> {
    cached: Mutex<CachedModel<M>>,
    lifecycle: Mutex<()>,
    unload_epoch: AtomicU64,
}

impl<M> ModelCache<M> {
    fn new() -> Self {
        Self {
            cached: Mutex::new(CachedModel {
                model: None,
                last_used: None,
                epoch: 0,
            }),
            lifecycle: Mutex::new(()),
            unload_epoch: AtomicU64::new(0),
        }
    }

    fn epoch(&self) -> u64 {
        self.unload_epoch.load(Ordering::Acquire)
    }

    fn with_model<A, L, F>(
        &self,
        request_epoch: u64,
        allowed: A,
        load_model: L,
        embed: F,
    ) -> Result<Vec<f32>, EmbedError>
    where
        A: Fn() -> bool,
        L: FnOnce() -> Result<M, String>,
        F: FnOnce(&M) -> Result<Vec<f32>, String>,
    {
        self.with_request(request_epoch, allowed, || true, load_model, embed)
    }

    fn with_request<A, R, L, F>(
        &self,
        request_epoch: u64,
        allowed: A,
        current: R,
        load_model: L,
        embed: F,
    ) -> Result<Vec<f32>, EmbedError>
    where
        A: Fn() -> bool,
        R: Fn() -> bool,
        L: FnOnce() -> Result<M, String>,
        F: FnOnce(&M) -> Result<Vec<f32>, String>,
    {
        let mut cached = self
            .cached
            .lock()
            .map_err(|_| EmbedError::Load("embedder mutex poisoned".to_string()))?;
        if cached.epoch != self.epoch() {
            cached.clear();
        }

        // Capture the request epoch before waiting on the cache. Work queued
        // before disable stays cancelled even if the user re-enables quickly.
        let mut did_embed = false;
        let mut did_load = false;
        let mut result = if !allowed() || !current() || request_epoch != self.epoch() {
            Err(EmbedError::Cancelled)
        } else {
            let loaded = if cached.model.is_none() {
                load_model().map(|model| {
                    cached.model = Some(model);
                    cached.epoch = request_epoch;
                    did_load = true;
                })
            } else {
                Ok(())
            };
            match loaded {
                Err(error) => Err(EmbedError::Load(error)),
                Ok(()) if !allowed() || !current() || request_epoch != self.epoch() => {
                    Err(EmbedError::Cancelled)
                }
                Ok(()) => match cached.model.as_ref() {
                    Some(model) => {
                        did_embed = true;
                        embed(model).map_err(EmbedError::Inference)
                    }
                    None => Err(EmbedError::Load("embedder unavailable".to_string())),
                },
            }
        };

        let lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let enabled = allowed();
        let current_epoch = self.epoch();
        if !enabled || !current() || request_epoch != current_epoch {
            result = Err(EmbedError::Cancelled);
        }
        if !enabled || cached.epoch != current_epoch {
            cached.clear();
        } else if did_embed || did_load {
            // Errors still count as activity: do not repeatedly unload/reload
            // the model while the worker skips individual malformed chunks.
            cached.last_used = Some(Instant::now());
        }
        // A disable request cannot arrive between the final check and unlock:
        // it either marked this active operation, or can take the free cache.
        drop(cached);
        drop(lifecycle);
        result
    }

    fn request_unload(&self) {
        let _lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.unload_epoch.fetch_add(1, Ordering::AcqRel);
        match self.cached.try_lock() {
            Ok(mut cached) => {
                cached.clear();
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().clear();
            }
            // Active loading/inference clears the model at completion.
            Err(TryLockError::WouldBlock) => {}
        }
    }

    fn evict_if_idle_at(&self, now: Instant) -> bool {
        let _lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Ok(mut cached) = self.cached.try_lock() else {
            return false;
        };
        if cached
            .last_used
            .is_some_and(|last| now.saturating_duration_since(last) >= MODEL_IDLE_TIMEOUT)
        {
            cached.clear()
        } else {
            false
        }
    }
}

static EMBEDDER: Lazy<ModelCache<Embedder>> = Lazy::new(ModelCache::new);

/// Capture before dispatching to a blocking pool. A query waiting to start in
/// that pool must stay cancelled across disable/re-enable, just like one
/// waiting for the model, without clearing a newer enabled model's cache.
pub fn request_epoch() -> u64 {
    EMBEDDER.epoch()
}

/// Release an unused model immediately, or ask an active operation to release
/// it on completion. Never waits for model loading or inference. The last
/// query's text and vector go too ([`QUERY_CACHE`], OF26 review): the new
/// epoch already keeps them from being reused, but a request's words
/// shouldn't stay in memory once semantic search is switched off. Cleared
/// after the model's lifecycle lock is released: the query cache's lock is
/// a leaf, never held while taking another.
pub fn request_unload() {
    EMBEDDER.request_unload();
    QUERY_CACHE.clear();
}

/// Called by the worker's maintenance tick, including while search is disabled.
/// Does not wait for an active embedding or reset its idle timer.
pub fn evict_if_idle() -> bool {
    EMBEDDER.evict_if_idle_at(Instant::now())
}

fn load(model_dir: &Path) -> Result<Embedder, String> {
    let device = Device::Cpu;
    let cfg_text = std::fs::read_to_string(model_dir.join("config.json"))
        .map_err(|e| format!("read config.json: {e}"))?;
    let cfg: Config =
        serde_json::from_str(&cfg_text).map_err(|e| format!("parse config.json: {e}"))?;
    let tokenizer = Tokenizer::from_file(model_dir.join("tokenizer.json"))
        .map_err(|e| format!("load tokenizer.json: {e}"))?;
    // mmap the f32 safetensors. `unsafe` is only because it memory-maps the
    // file (the standard candle idiom). F32 on CPU is the supported bert path;
    // f16-on-CPU falls to a slow scalar matmul, so stay at DTYPE (== F32).
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[model_dir.join("model.safetensors")], DTYPE, &device)
            .map_err(|e| format!("load safetensors: {e}"))?
    };
    let model = BertModel::load(vb, &cfg).map_err(|e| format!("build bert: {e}"))?;
    Ok(Embedder {
        model,
        tokenizer,
        device,
    })
}

fn forward_cls(e: &Embedder, text: &str) -> Result<Vec<f32>, String> {
    let enc = e
        .tokenizer
        .encode(text, true)
        .map_err(|e| format!("encode: {e}"))?;
    let mut ids: Vec<u32> = enc.get_ids().to_vec();
    let mut mask: Vec<u32> = enc.get_attention_mask().to_vec();
    // Cap at the model's position-embedding limit. [CLS] is at index 0, so
    // tail truncation keeps the pooled token intact.
    if ids.len() > MAX_TOKENS {
        ids.truncate(MAX_TOKENS);
        mask.truncate(MAX_TOKENS);
    }
    let input_ids = Tensor::new(ids.as_slice(), &e.device)
        .and_then(|t| t.unsqueeze(0))
        .map_err(|e| format!("input_ids: {e}"))?; // [1, n]
                                                  // token_type_ids is a REQUIRED Tensor (not Option) — all-zeros for a single
                                                  // sentence; matches input_ids shape + dtype.
    let token_type_ids = input_ids
        .zeros_like()
        .map_err(|e| format!("token_type_ids: {e}"))?;
    let attn = Tensor::new(mask.as_slice(), &e.device)
        .and_then(|t| t.unsqueeze(0))
        .map_err(|e| format!("attention_mask: {e}"))?; // [1, n]
    let hidden = e
        .model
        .forward(&input_ids, &token_type_ids, Some(&attn))
        .map_err(|e| format!("forward: {e}"))?; // [1, n, 384]
                                                // bge → CLS pooling: take token position 0. NOT mean pooling (that is the
                                                // all-MiniLM recipe; applying it to bge silently degrades recall).
    let cls = hidden.i((.., 0)).map_err(|e| format!("cls pool: {e}"))?; // [1, 384]
                                                                        // L2-normalize so downstream cosine similarity == dot product.
    let norm = cls
        .sqr()
        .and_then(|t| t.sum_keepdim(1))
        .and_then(|t| t.sqrt())
        .map_err(|e| format!("norm: {e}"))?; // [1, 1]
    let normed = cls
        .broadcast_div(&norm)
        .map_err(|e| format!("normalize: {e}"))?; // [1, 384]
    let v: Vec<f32> = normed
        .squeeze(0)
        .and_then(|t| t.to_vec1::<f32>())
        .map_err(|e| format!("to_vec: {e}"))?;
    Ok(v)
}

fn with_embedder<A, F>(model_dir: &Path, allowed: A, f: F) -> Result<Vec<f32>, EmbedError>
where
    A: Fn() -> bool,
    F: FnOnce(&Embedder) -> Result<Vec<f32>, String>,
{
    EMBEDDER.with_model(EMBEDDER.epoch(), allowed, || load(model_dir), f)
}

/// Load the opted-in model before the first query, without inventing a
/// query vector. The same epoch and disable guards as inference apply.
pub(crate) fn warm_on_enable() {
    static WARMING: AtomicBool = AtomicBool::new(false);
    if cfg!(test) || WARMING.swap(true, Ordering::AcqRel) {
        return;
    }
    struct WarmGuard;
    impl Drop for WarmGuard {
        fn drop(&mut self) {
            WARMING.store(false, Ordering::Release);
        }
    }
    let guard = WarmGuard;
    let epoch = request_epoch();
    let _ = std::thread::Builder::new()
        .name("hollow-embed-warm".into())
        .spawn(move || {
            let _guard = guard;
            let dir = default_model_dir();
            let _ = EMBEDDER.with_model(
                epoch,
                crate::vault_embed_index::is_semantic_enabled,
                || load(&dir),
                |_| Ok(Vec::new()),
            );
        });
}

/// Embed a passage / chunk for the index (no instruction prefix).
pub fn embed_passage(model_dir: &Path, text: &str) -> Result<Vec<f32>, String> {
    embed_passage_if_enabled(model_dir, text, || true).map_err(|error| error.to_string())
}

/// Embed a search query (bge instruction prefix prepended).
pub fn embed_query(model_dir: &Path, text: &str) -> Result<Vec<f32>, String> {
    embed_query_if_enabled(model_dir, text, || true).map_err(|error| error.to_string())
}

/// App passage entrypoint. Rechecks enabled state after waiting for another
/// embedding, after loading, and at completion; cancelled work stays pending.
pub fn embed_passage_if_enabled<A>(
    model_dir: &Path,
    text: &str,
    allowed: A,
) -> Result<Vec<f32>, EmbedError>
where
    A: Fn() -> bool,
{
    with_embedder(model_dir, allowed, |e| forward_cls(e, text))
}

/// App query entrypoint with the same lifecycle guards as background passages.
pub fn embed_query_if_enabled<A>(
    model_dir: &Path,
    text: &str,
    allowed: A,
) -> Result<Vec<f32>, EmbedError>
where
    A: Fn() -> bool,
{
    embed_query_for_request(model_dir, text, request_epoch(), allowed)
}

/// Characters of a request embedded as a query (the model reads 512 tokens
/// anyway). Recall (OF26, `TurnRequest::meaning_query`) and retrieval
/// (`retrieval_broker::SEMANTIC_QUERY_CHARS`, the same 2,000) must cut a
/// request alike for [`QUERY_CACHE`] to serve both (pinned by
/// `tests::recall_and_retrieval_cut_a_query_alike`).
pub const QUERY_CHARS: usize = 2_000;

/// One embedded query, as [`QueryCache`] keeps it.
struct CachedQuery {
    model_dir: PathBuf,
    epoch: u64,
    text: String,
    vector: Vec<f32>,
}

/// The last query embedded (OF26): a chat turn's recall embeds its query
/// first, and retrieval's semantic half then reads the vector from here
/// instead of embedding the same text again. One entry, keyed on the exact
/// text, the model directory and the unload epoch (a disable, or a disable
/// and re-enable, starts it over). Its lock is a leaf: taken briefly, never
/// held while waiting for the model, so taking it inside the model's lock
/// ([`embed_query_through`]) can't deadlock.
struct QueryCache {
    last: Mutex<Option<CachedQuery>>,
}

impl QueryCache {
    const fn new() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }

    fn lookup(&self, model_dir: &Path, epoch: u64, text: &str) -> Option<Vec<f32>> {
        let last = self.last.lock().ok()?;
        let hit = last.as_ref()?;
        let same = hit.epoch == epoch && hit.text == text && hit.model_dir.as_path() == model_dir;
        same.then(|| hit.vector.clone())
    }

    /// `vector` as the last query, in place of the entry before it.
    fn store(&self, model_dir: &Path, epoch: u64, text: &str, vector: &[f32]) {
        if let Ok(mut last) = self.last.lock() {
            *last = Some(CachedQuery {
                model_dir: model_dir.to_path_buf(),
                epoch,
                text: text.to_string(),
                vector: vector.to_vec(),
            });
        }
    }

    fn clear(&self) {
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last = None;
    }
}

static QUERY_CACHE: QueryCache = QueryCache::new();

/// Query with an epoch captured before dispatch to the blocking pool. The
/// same text again, from the same model and epoch, is answered from
/// [`QUERY_CACHE`] without inference, under the same guards: a disabled or
/// stale request is still cancelled.
pub fn embed_query_for_request<A>(
    model_dir: &Path,
    text: &str,
    request_epoch: u64,
    allowed: A,
) -> Result<Vec<f32>, EmbedError>
where
    A: Fn() -> bool,
{
    embed_query_preparing(model_dir, text, request_epoch, allowed, || true)
}

/// Request cancellation discards this result without disabling shared caches.
pub(crate) fn embed_query_preparing<A, R>(
    model_dir: &Path,
    text: &str,
    request_epoch: u64,
    allowed: A,
    current: R,
) -> Result<Vec<f32>, EmbedError>
where
    A: Fn() -> bool,
    R: Fn() -> bool,
{
    let q = format!("{QUERY_PREFIX}{text}");
    embed_query_through_current(
        &QUERY_CACHE,
        &*EMBEDDER,
        model_dir,
        text,
        request_epoch,
        allowed,
        current,
        || load(model_dir),
        |e| forward_cls(e, &q),
    )
}

/// [`embed_query_for_request`] over the caches given (tests: fakes). The
/// cache is read before waiting for the model and again once the model is
/// held (OF26 review): a recall whose bounded wait ran out leaves its
/// embedding running (`vault_embed_index::EngagedMeaning`), and retrieval,
/// or a workflow's recall after its retrieval, that queued behind it on the
/// model then takes its vector instead of embedding the same text again. A
/// fresh vector is stored while the model is still held, so nothing queued
/// behind it can miss it. The second read runs only once `with_model` has
/// checked that the request is allowed and of the current epoch.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn embed_query_through<M, A, L, F>(
    queries: &QueryCache,
    models: &ModelCache<M>,
    model_dir: &Path,
    text: &str,
    request_epoch: u64,
    allowed: A,
    load_model: L,
    forward: F,
) -> Result<Vec<f32>, EmbedError>
where
    A: Fn() -> bool,
    L: FnOnce() -> Result<M, String>,
    F: FnOnce(&M) -> Result<Vec<f32>, String>,
{
    embed_query_through_current(
        queries,
        models,
        model_dir,
        text,
        request_epoch,
        allowed,
        || true,
        load_model,
        forward,
    )
}

#[allow(clippy::too_many_arguments)]
fn embed_query_through_current<M, A, R, L, F>(
    queries: &QueryCache,
    models: &ModelCache<M>,
    model_dir: &Path,
    text: &str,
    request_epoch: u64,
    allowed: A,
    current: R,
    load_model: L,
    forward: F,
) -> Result<Vec<f32>, EmbedError>
where
    A: Fn() -> bool,
    R: Fn() -> bool,
    L: FnOnce() -> Result<M, String>,
    F: FnOnce(&M) -> Result<Vec<f32>, String>,
{
    if allowed() && current() && request_epoch == models.epoch() {
        if let Some(vector) = queries.lookup(model_dir, request_epoch, text) {
            return Ok(vector);
        }
    }
    models.with_request(request_epoch, &allowed, &current, load_model, |model| {
        if let Some(vector) = queries.lookup(model_dir, request_epoch, text) {
            return Ok(vector);
        }
        let vector = forward(model)?;
        // Not once a disable has cleared the cache: the text would outlive it.
        if allowed() && current() && request_epoch == models.epoch() {
            queries.store(model_dir, request_epoch, text, &vector);
            // A disable between that check and the store (second review):
            // its clear may have run first, so clear again. An unload bumps
            // the epoch before it clears, so one that hasn't bumped it yet
            // clears after this store.
            if request_epoch != models.epoch() {
                queries.clear();
            }
        }
        Ok(vector)
    })
}

/// A private model instance outside the process cache, for evaluations that
/// must not race the cache's own tests (loading, eviction, epochs).
#[cfg(test)]
pub(crate) struct OwnEmbedder(Embedder);

#[cfg(test)]
impl OwnEmbedder {
    pub(crate) fn load(model_dir: &Path) -> Result<Self, String> {
        load(model_dir).map(OwnEmbedder)
    }

    /// As [`embed_passage`].
    pub(crate) fn passage(&self, text: &str) -> Result<Vec<f32>, String> {
        forward_cls(&self.0, text)
    }

    /// As [`embed_query`]: the bge instruction prefix first.
    pub(crate) fn query(&self, text: &str) -> Result<Vec<f32>, String> {
        forward_cls(&self.0, &format!("{QUERY_PREFIX}{text}"))
    }
}

/// Resolve the bundled model dir. Inside `Hollow.app` this is
/// `Contents/Resources/models/bge-small-en-v1.5`; a dev / `cargo test` run
/// falls back to `CARGO_MANIFEST_DIR/models/bge-small-en-v1.5`.
pub fn default_model_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let res = parent.join("../Resources/models/bge-small-en-v1.5");
            if res.join("model.safetensors").exists() {
                return res;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/bge-small-en-v1.5")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::{mpsc, Arc};

    struct FakeModel {
        drops: Arc<AtomicUsize>,
    }

    impl Drop for FakeModel {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn fake_load(loads: &AtomicUsize, drops: &Arc<AtomicUsize>) -> Result<FakeModel, String> {
        loads.fetch_add(1, Ordering::SeqCst);
        Ok(FakeModel {
            drops: Arc::clone(drops),
        })
    }

    #[test]
    fn cache_is_lazy_reused_then_evicted_and_reloaded() {
        let cache = ModelCache::new();
        let loads = AtomicUsize::new(0);
        let drops = Arc::new(AtomicUsize::new(0));
        assert!(!cache.evict_if_idle_at(Instant::now() + MODEL_IDLE_TIMEOUT));
        assert_eq!(loads.load(Ordering::SeqCst), 0);

        for _ in 0..2 {
            assert_eq!(
                cache.with_model(
                    cache.epoch(),
                    || true,
                    || fake_load(&loads, &drops),
                    |_| Ok(vec![1.0]),
                ),
                Ok(vec![1.0])
            );
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        let last_used = cache.cached.lock().unwrap().last_used.unwrap();
        assert!(!cache.evict_if_idle_at(last_used + MODEL_IDLE_TIMEOUT - Duration::from_nanos(1)));
        assert!(cache.evict_if_idle_at(last_used + MODEL_IDLE_TIMEOUT));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(!cache.evict_if_idle_at(last_used + MODEL_IDLE_TIMEOUT));

        assert_eq!(
            cache.with_model(
                cache.epoch(),
                || true,
                || fake_load(&loads, &drops),
                |_| Ok(vec![2.0]),
            ),
            Ok(vec![2.0])
        );
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        cache.request_unload();
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert!(cache.cached.lock().unwrap().model.is_none());
    }

    #[test]
    fn disabled_requests_never_load_and_failures_remain_distinct() {
        let cache = ModelCache::new();
        let loads = AtomicUsize::new(0);
        let drops = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            cache.with_model(
                cache.epoch(),
                || false,
                || fake_load(&loads, &drops),
                |_| Ok(vec![]),
            ),
            Err(EmbedError::Cancelled)
        );
        assert_eq!(loads.load(Ordering::SeqCst), 0);
        assert_eq!(
            cache.with_model(
                cache.epoch(),
                || true,
                || Err("missing weights".to_string()),
                |_| Ok(vec![]),
            ),
            Err(EmbedError::Load("missing weights".to_string()))
        );
        assert!(cache.cached.lock().unwrap().model.is_none());
        assert_eq!(
            cache.with_model(
                cache.epoch(),
                || true,
                || fake_load(&loads, &drops),
                |_| Err("bad input".to_string()),
            ),
            Err(EmbedError::Inference("bad input".to_string()))
        );
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancelled_preparation_retains_shared_model_and_query() {
        let queries = QueryCache::new();
        let models = ModelCache::new();
        let dir = Path::new("/fake-model");
        let loads = AtomicUsize::new(0);
        let drops = Arc::new(AtomicUsize::new(0));
        let current = AtomicBool::new(true);
        assert_eq!(
            embed_query_through_current(
                &queries,
                &models,
                dir,
                "query",
                models.epoch(),
                || true,
                || current.load(Ordering::Acquire),
                || fake_load(&loads, &drops),
                |_| Ok(vec![1.0]),
            ),
            Ok(vec![1.0])
        );
        current.store(false, Ordering::Release);
        assert_eq!(
            embed_query_through_current(
                &queries,
                &models,
                dir,
                "query",
                models.epoch(),
                || true,
                || current.load(Ordering::Acquire),
                || panic!("cancelled request loaded"),
                |_| panic!("cancelled request inferred"),
            ),
            Err(EmbedError::Cancelled)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(
            queries.lookup(dir, models.epoch(), "query"),
            Some(vec![1.0])
        );
        assert_eq!(
            models.with_request(
                models.epoch(),
                || true,
                || true,
                || panic!("shared model reloaded"),
                |_| Ok(vec![2.0])
            ),
            Ok(vec![2.0])
        );
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancellation_during_load_retains_model_with_idle_deadline() {
        let cache = ModelCache::new();
        let current = AtomicBool::new(true);
        let loads = AtomicUsize::new(0);
        let drops = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            cache.with_request(
                cache.epoch(),
                || true,
                || current.load(Ordering::Acquire),
                || {
                    let model = fake_load(&loads, &drops)?;
                    current.store(false, Ordering::Release);
                    Ok(model)
                },
                |_| panic!("cancelled load inferred"),
            ),
            Err(EmbedError::Cancelled)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let last_used = cache.cached.lock().unwrap().last_used.unwrap();
        assert!(cache.evict_if_idle_at(last_used + MODEL_IDLE_TIMEOUT));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn disable_during_loading_prevents_inference_and_releases_model() {
        let cache = ModelCache::new();
        let enabled = AtomicBool::new(true);
        let loads = AtomicUsize::new(0);
        let drops = Arc::new(AtomicUsize::new(0));
        let inferences = AtomicUsize::new(0);
        assert_eq!(
            cache.with_model(
                cache.epoch(),
                || enabled.load(Ordering::Acquire),
                || {
                    let model = fake_load(&loads, &drops)?;
                    enabled.store(false, Ordering::Release);
                    cache.request_unload();
                    Ok(model)
                },
                |_| {
                    inferences.fetch_add(1, Ordering::SeqCst);
                    Ok(vec![])
                },
            ),
            Err(EmbedError::Cancelled)
        );
        assert_eq!(inferences.load(Ordering::SeqCst), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(cache.cached.lock().unwrap().model.is_none());
    }

    #[test]
    fn disable_returns_while_inference_is_busy_and_completion_releases_model() {
        let cache = Arc::new(ModelCache::new());
        let enabled = Arc::new(AtomicBool::new(true));
        let drops = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel();
        let running = {
            let cache = Arc::clone(&cache);
            let enabled = Arc::clone(&enabled);
            let drops = Arc::clone(&drops);
            std::thread::spawn(move || {
                cache.with_model(
                    cache.epoch(),
                    || enabled.load(Ordering::Acquire),
                    || Ok(FakeModel { drops }),
                    |_| {
                        started_tx.send(()).unwrap();
                        finish_rx.recv().unwrap();
                        Ok(vec![1.0])
                    },
                )
            })
        };
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!cache.evict_if_idle_at(Instant::now() + MODEL_IDLE_TIMEOUT));
        let (unloaded_tx, unloaded_rx) = mpsc::channel();
        let disabling = {
            let cache = Arc::clone(&cache);
            let enabled = Arc::clone(&enabled);
            std::thread::spawn(move || {
                enabled.store(false, Ordering::Release);
                cache.request_unload();
                unloaded_tx.send(()).unwrap();
            })
        };
        let returned_while_busy = unloaded_rx.recv_timeout(Duration::from_secs(2));
        let drops_while_busy = drops.load(Ordering::SeqCst);
        // Always unblock the worker, even if a regression makes unload wait.
        finish_tx.send(()).unwrap();
        disabling.join().unwrap();
        assert_eq!(running.join().unwrap(), Err(EmbedError::Cancelled));
        assert!(returned_while_busy.is_ok(), "disable waited for inference");
        assert_eq!(drops_while_busy, 0);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(cache.cached.lock().unwrap().model.is_none());
    }

    #[test]
    fn queued_work_stays_cancelled_after_disable_and_reenable() {
        let cache = ModelCache::new();
        let loads = AtomicUsize::new(0);
        let drops = Arc::new(AtomicUsize::new(0));
        // The guarded entrypoint captures this before waiting for the cache.
        let queued_epoch = cache.epoch();
        cache.request_unload();
        assert_eq!(
            cache.with_model(
                cache.epoch(),
                || true,
                || fake_load(&loads, &drops),
                |_| Ok(vec![1.0]),
            ),
            Ok(vec![1.0])
        );
        // Stale queued work neither runs nor evicts a newer, enabled model.
        assert_eq!(
            cache.with_model(
                queued_epoch,
                || true,
                || fake_load(&loads, &drops),
                |_| panic!("stale queued embedding ran"),
            ),
            Err(EmbedError::Cancelled)
        );
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(cache.cached.lock().unwrap().model.is_some());
    }

    /// Red first (OF26): recall and retrieval each embedded a turn's query,
    /// so the same text ran through the model twice. The one-entry cache
    /// answers a repeat of the last text, model and epoch without calling
    /// the embedder; another text replaces the entry, an unload's new epoch
    /// or another model misses, a disabled request for exactly the cached
    /// key is still cancelled (review: the old case used another key, so it
    /// passed even with the guard ignored), a failure is never cached, and
    /// a cleared cache (`request_unload`) embeds again.
    #[test]
    fn the_query_cache_embeds_a_repeated_text_once() {
        let queries = QueryCache::new();
        let models: ModelCache<()> = ModelCache::new();
        let bge = Path::new("/models/bge-small-en-v1.5");
        let other = Path::new("/models/other");
        let embeds = AtomicUsize::new(0);
        let get = |dir: &Path, text: &str, allowed: bool| {
            embed_query_through(
                &queries,
                &models,
                dir,
                text,
                models.epoch(),
                move || allowed,
                || Ok(()),
                |_: &()| {
                    embeds.fetch_add(1, Ordering::SeqCst);
                    Ok(vec![text.len() as f32])
                },
            )
        };
        let query = "when can we ship the release";
        assert_eq!(get(bge, query, true), Ok(vec![28.0]));
        assert_eq!(get(bge, query, true), Ok(vec![28.0]));
        assert_eq!(embeds.load(Ordering::SeqCst), 1, "the repeat is cached");

        assert_eq!(get(bge, "deploy rules", true), Ok(vec![12.0]));
        assert_eq!(get(bge, query, true), Ok(vec![28.0]));
        assert_eq!(embeds.load(Ordering::SeqCst), 3, "one entry only");

        models.request_unload();
        assert_eq!(get(bge, query, true), Ok(vec![28.0]));
        assert_eq!(embeds.load(Ordering::SeqCst), 4, "an unload starts over");
        assert_eq!(get(other, query, true), Ok(vec![28.0]));
        assert_eq!(embeds.load(Ordering::SeqCst), 5, "another model misses");

        // The cache now holds exactly (other, this epoch, query).
        assert_eq!(
            get(other, query, false),
            Err(EmbedError::Cancelled),
            "disabled"
        );
        assert_eq!(embeds.load(Ordering::SeqCst), 5);

        let failed = embed_query_through(
            &queries,
            &models,
            bge,
            "fails",
            models.epoch(),
            || true,
            || Ok(()),
            |_: &()| Err("bad input".to_string()),
        );
        assert_eq!(failed, Err(EmbedError::Inference("bad input".into())));
        assert_eq!(get(bge, "fails", true), Ok(vec![5.0]), "not cached");
        assert_eq!(embeds.load(Ordering::SeqCst), 6);

        assert_eq!(get(bge, "fails", true), Ok(vec![5.0]));
        assert_eq!(embeds.load(Ordering::SeqCst), 6);
        queries.clear();
        assert_eq!(get(bge, "fails", true), Ok(vec![5.0]));
        assert_eq!(
            embeds.load(Ordering::SeqCst),
            7,
            "a cleared cache embeds again"
        );
    }

    /// Red first (OF26 review, B1): a turn's recall stops waiting for its
    /// query vector after its wait (`RECALL_MEANING_WAIT` or the shorter `RECALL_SLOW_WAIT`) and leaves the embedding
    /// running. Retrieval then asks for the same text, misses the cache
    /// (nothing stored yet) and queues on the model behind it. Before, it
    /// embedded the text again once the model was free; now it reads the
    /// cache again once it holds the model and takes the vector embedded
    /// ahead of it.
    #[test]
    fn a_query_queued_on_the_model_takes_the_vector_embedded_ahead_of_it() {
        let queries = &QueryCache::new();
        let models: &ModelCache<()> = &ModelCache::new();
        let dir = Path::new("/models/bge-small-en-v1.5");
        let text = "when can we ship the release";
        let epoch = models.epoch();
        let again = &AtomicUsize::new(0);
        let (started_tx, started_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel::<()>();
        std::thread::scope(|s| {
            let first = s.spawn(move || {
                embed_query_through(
                    queries,
                    models,
                    dir,
                    text,
                    epoch,
                    || true,
                    || Ok(()),
                    |_: &()| {
                        started_tx.send(()).unwrap();
                        finish_rx.recv().unwrap();
                        Ok(vec![1.0])
                    },
                )
            });
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let second = s.spawn(move || {
                embed_query_through(
                    queries,
                    models,
                    dir,
                    text,
                    epoch,
                    || true,
                    || Ok(()),
                    |_: &()| {
                        again.fetch_add(1, Ordering::SeqCst);
                        Ok(vec![2.0])
                    },
                )
            });
            // Let the second caller miss the cache and queue on the model.
            std::thread::sleep(Duration::from_millis(150));
            finish_tx.send(()).unwrap();
            assert_eq!(first.join().unwrap(), Ok(vec![1.0]));
            assert_eq!(second.join().unwrap(), Ok(vec![1.0]));
        });
        assert_eq!(again.load(Ordering::SeqCst), 0, "embedded twice");
    }

    /// OF26 review (M2): the one embedding per turn holds only while recall
    /// cuts a request exactly as retrieval does; retrieval's constant is
    /// private, so its text is pinned here.
    #[test]
    fn recall_and_retrieval_cut_a_query_alike() {
        let broker = include_str!("retrieval_broker.rs");
        let cut = "let text: String = query.chars().take(SEMANTIC_QUERY_CHARS).collect();";
        assert!(broker.contains("const SEMANTIC_QUERY_CHARS: usize = 2_000;"));
        assert!(broker.contains(cut));
        assert_eq!(QUERY_CHARS, 2_000);
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn embed_dim_norm_and_ranking() {
        let dir = default_model_dir();
        if !dir.join("model.safetensors").exists() {
            eprintln!(
                "bge model not fetched at {}; skipping embed test",
                dir.display()
            );
            return;
        }
        let v = embed_passage(&dir, "hello world").expect("embed failed");
        assert_eq!(v.len(), 384, "expected 384 dims");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "not L2-normalized: {norm}");

        // Ranking sanity: a paraphrase must out-score an unrelated sentence.
        // This catches a wrong pooling choice, which the norm check cannot.
        let q = embed_query(&dir, "How do I reset my password?").unwrap();
        let near = embed_passage(&dir, "Steps to recover or reset your account password.").unwrap();
        let far = embed_passage(&dir, "The chef seared the scallops in brown butter.").unwrap();
        let s_near = cosine(&q, &near);
        let s_far = cosine(&q, &far);
        assert!(
            s_near > s_far,
            "semantic ranking wrong: near={s_near} far={s_far}"
        );

        request_unload();
        assert!(EMBEDDER.cached.lock().unwrap().model.is_none());
        let reloaded = embed_passage(&dir, "hello world").expect("reload failed");
        assert_eq!(reloaded.len(), 384);
        assert!(
            cosine(&v, &reloaded) > 0.9999,
            "lazy reload changed the passage embedding"
        );
        let reloaded_query = embed_query(&dir, "How do I reset my password?").unwrap();
        assert!(cosine(&reloaded_query, &near) > cosine(&reloaded_query, &far));
        request_unload();
    }
}
