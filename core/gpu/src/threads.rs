//! Worker threads of the 3D rasterizer and the hi-res upscaler.
//!
//! A dedicated pool instead of rayon's global one, which starts a thread per
//! core: every extra thread adds spinning and hand-off cost for the few
//! milliseconds of work per frame, so CPU time grows faster than the
//! rendering gets done (Zelda Spirit Tracks, 1800 frames: 1 thread 22.6 s
//! CPU, 16 threads 27.6 s CPU for 3.6 s less wall time).
//!
//! ```text
//!  set_render_threads(n)   n = 1: render on the emulator thread, no pool
//!                          n > 1: pool of n threads, rebuilt when n changes
//!  run(f)                  f runs inside the pool (its par_iter use it)
//! ```
use std::sync::{Arc, RwLock};

/// Threads used when the frontend never sets a count.
pub const DEFAULT_RENDER_THREADS: usize = 2;

static POOL: RwLock<Option<(usize, Option<Arc<rayon::ThreadPool>>)>> = RwLock::new(None);

/// Sets how many threads render 3D frames and upscale screens (1 = on the
/// emulator thread only). Takes effect from the next frame.
pub fn set_render_threads(n: usize) {
    let n = n.max(1);
    let mut pool = POOL.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if pool.as_ref().is_some_and(|(count, _)| *count == n) {
        return;
    }
    let built = (n > 1)
        .then(|| rayon::ThreadPoolBuilder::new().num_threads(n).build().ok().map(Arc::new))
        .flatten();
    *pool = Some((n, built));
}

/// Current thread count.
pub fn render_threads() -> usize {
    POOL.read().ok().and_then(|p| p.as_ref().map(|(n, _)| *n)).unwrap_or(DEFAULT_RENDER_THREADS)
}

/// Whether rendering is spread over a pool (`run` then uses it).
pub(crate) fn parallel() -> bool {
    render_threads() > 1
}

/// Runs `f` inside the render pool (rayon's global pool if it could not
/// be built). Callers check [`parallel`] first and stay on their own
/// thread when it is false.
pub(crate) fn run<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    if POOL.read().map(|p| p.is_none()).unwrap_or(true) {
        set_render_threads(DEFAULT_RENDER_THREADS);
    }
    let pool = POOL.read().ok().and_then(|p| p.as_ref().and_then(|(_, pool)| pool.clone()));
    match pool {
        Some(pool) => pool.install(f),
        None => f(),
    }
}
