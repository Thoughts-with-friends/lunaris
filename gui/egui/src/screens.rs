//! Owns the two egui textures the NDS screens are uploaded into every frame.
//!
//! See `docs/design/egui-migration-design.md` §5.2. Two separate textures
//! (rather than one stacked 256x384 texture, as the imgui front end uses)
//! so the Horizon (side-by-side) layout is a pure placement decision.
//!
//! Optional post-process upscaling (`docs/design/resolution-upscaling-design.md`)
//! is applied here, between the ABGR1555->RGBA8 conversion and the texture
//! upload, so the rest of the front end (layout, touch input, savestates)
//! never needs to know the on-screen texture resolution differs from the
//! native 256x192 NDS screen size.
//!
//! Upscaling runs on two dedicated background threads ([`ScreenWorker`]),
//! spawned once in [`ScreenTextures::new`] and never again — an earlier
//! version spawned a fresh thread per job, which measurably stalled the
//! main thread via Windows thread-creation/heap-lock contention.
//!
//! Even with the compute off the main thread, profiling (with
//! `[perf]`-tagged `eprintln!`s, since removed) showed a *second* main-thread
//! cost: applying a large upscaled result to an egui texture
//! (`ColorImage::from_rgba_unmultiplied` + `TextureHandle::set`) took
//! 30-45ms per screen at xBRZ 16x (4096x3072, ~50MB), because that work ran
//! in the same call as `NDS::emulate_frame()` — dragging the reported
//! emulate-loop FPS down even though `emulate_frame()` itself only cost
//! 6-10ms. Almost all of that 50MB was wasted: a normal window only shows
//! roughly 800px per screen, so the GPU downscales nearly everything away.
//! [`effective_factor`] caps the factor actually computed/uploaded to what
//! the on-screen size can show, independent of the user's nominal setting —
//! visually identical (upscaling past display resolution is invisible) but
//! bounds the cost by window size instead of by the slider value.

use std::sync::{Arc, Condvar, Mutex};

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use lunaris_gui_common::framebuffer::{SCREEN_HEIGHT, SCREEN_WIDTH, abgr1555_to_rgba8};
use lunaris_gui_common::upscale::{self, UpscaleMethod};
use nds_core::nds::NDS;

/// One request to convert and upscale a single screen's raw pixels. Builds
/// the final [`ColorImage`] on the worker too (not just the raw upscale),
/// so the only work left for the main thread is staging the already-built
/// image into the texture.
struct ScreenJob {
    pixels: Vec<u16>,
    method: UpscaleMethod,
    factor: u8,
}

/// Runs [`ScreenJob`]s for one screen on a single persistent background
/// thread, always working on the newest submitted job. See the module docs
/// for why the thread is spawned exactly once and never again.
///
/// The "newest job wins" mailbox (as opposed to a FIFO queue) is
/// deliberate: if the worker can't keep up with the emulation rate at a
/// high factor, older pending frames are simply dropped rather than
/// queuing up and making the displayed image increasingly stale.
struct ScreenWorker {
    job_slot: Arc<(Mutex<Option<ScreenJob>>, Condvar)>,
    result_slot: Arc<Mutex<Option<ColorImage>>>,
    // Kept only to document ownership; the worker thread runs for the
    // lifetime of the process and is never joined (Rust does not join
    // background threads on `main` returning, so this is not a leak).
    _handle: std::thread::JoinHandle<()>,
}

impl ScreenWorker {
    fn spawn(thread_name: &'static str) -> Self {
        let job_slot = Arc::new((Mutex::new(None::<ScreenJob>), Condvar::new()));
        let result_slot = Arc::new(Mutex::new(None::<ColorImage>));

        let worker_jobs = Arc::clone(&job_slot);
        let worker_results = Arc::clone(&result_slot);
        let handle = std::thread::Builder::new()
            .name(thread_name.to_owned())
            .spawn(move || Self::run(&worker_jobs, &worker_results))
            .expect("failed to spawn screen upscale worker thread");

        ScreenWorker { job_slot, result_slot, _handle: handle }
    }

    /// Replaces any not-yet-started pending job with `job` and wakes the
    /// worker. Never blocks on the worker itself (only briefly on the
    /// mailbox mutex), so this is safe to call every emulated frame from
    /// the main/UI thread.
    fn submit(&self, job: ScreenJob) {
        let (mutex, condvar) = &*self.job_slot;
        let mut slot = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(job);
        condvar.notify_one();
    }

    /// Returns the most recently finished result, if one completed since
    /// the last call. Never blocks.
    fn try_take_result(&self) -> Option<ColorImage> {
        self.result_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }

    fn run(
        job_slot: &(Mutex<Option<ScreenJob>>, Condvar),
        result_slot: &Mutex<Option<ColorImage>>,
    ) {
        let (mutex, condvar) = job_slot;
        loop {
            let job = {
                let mut slot = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                while slot.is_none() {
                    slot = condvar.wait(slot).unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                // `while slot.is_none()` above only exits once `slot` holds
                // a value, so this always succeeds.
                match slot.take() {
                    Some(job) => job,
                    None => continue,
                }
            };

            let rgba = abgr1555_to_rgba8(&job.pixels);
            let (buf, w, h) =
                upscale::upscale(rgba, SCREEN_WIDTH, SCREEN_HEIGHT, job.method, job.factor);
            let image = ColorImage::from_rgba_unmultiplied([w, h], &buf);
            *result_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(image);
        }
    }
}

pub struct ScreenTextures {
    pub top: TextureHandle,
    pub bottom: TextureHandle,
    top_worker: ScreenWorker,
    bottom_worker: ScreenWorker,
    /// The factor actually submitted to the workers on the most recent
    /// dirty call, after [`effective_factor`] capped it to the display
    /// size. Exposed so the Video window can show the user an honest
    /// output size instead of the nominal (possibly much larger) setting.
    last_effective_factor: u8,
}

/// Clamps `factor` down to the largest value that keeps `256 * factor`
/// within `max_texture_side`, so a very large user-selected factor can never
/// request a texture bigger than the active egui backend supports.
///
/// See `docs/design/resolution-upscaling-design.md` §5.3.
fn clamp_factor_to_texture_limit(factor: u8, max_texture_side: usize) -> u8 {
    let max_from_backend = (max_texture_side / SCREEN_WIDTH.max(SCREEN_HEIGHT)) as u8;
    factor.min(max_from_backend.max(upscale::MIN_FACTOR))
}

/// Caps `nominal_factor` (the user's Video-window slider value) down to the
/// smallest factor whose output is still at least as large as the screen's
/// actual on-screen size, `display_px` (physical pixels, i.e. already
/// multiplied by `pixels_per_point`). Never exceeds `nominal_factor` itself.
///
/// Upscaling past what the display can show is invisible after the GPU
/// downscales it back down for painting, so this trades away only wasted
/// compute/upload cost, not visual quality. See the module docs.
fn effective_factor(nominal_factor: u8, display_px: f32, max_texture_side: usize) -> u8 {
    let needed = (display_px / SCREEN_WIDTH as f32).ceil();
    // `needed` is finite and >= 0 for any sane window size; NaN/negative
    // (a not-yet-laid-out panel reporting 0 or garbage size) falls back to
    // the smallest useful factor rather than propagating into `as u8`.
    let needed_factor = if needed.is_finite() && needed >= 1.0 {
        needed.min(u8::MAX as f32) as u8
    } else {
        upscale::MIN_FACTOR
    };
    let capped = nominal_factor.min(needed_factor).max(upscale::MIN_FACTOR);
    clamp_factor_to_texture_limit(capped, max_texture_side)
}

impl ScreenTextures {
    pub fn new(ctx: &Context) -> Self {
        let placeholder = ColorImage::new(
            [SCREEN_WIDTH, SCREEN_HEIGHT],
            vec![egui::Color32::BLACK; SCREEN_WIDTH * SCREEN_HEIGHT],
        );
        ScreenTextures {
            top: ctx.load_texture("nds-top-screen", placeholder.clone(), TextureOptions::NEAREST),
            bottom: ctx.load_texture("nds-bottom-screen", placeholder, TextureOptions::NEAREST),
            top_worker: ScreenWorker::spawn("upscale-top"),
            bottom_worker: ScreenWorker::spawn("upscale-bottom"),
            last_effective_factor: upscale::MIN_FACTOR,
        }
    }

    /// The factor actually computed/uploaded as of the most recent dirty
    /// [`Self::update`] call. See [`effective_factor`].
    pub const fn last_effective_factor(&self) -> u8 {
        self.last_effective_factor
    }

    /// Submits the current frame's raw pixels to the background workers
    /// (when `dirty`), and applies whichever upscaled results have most
    /// recently finished to the textures. Never blocks on either worker —
    /// see the module docs for why.
    ///
    /// `dirty` should be `false` when neither the emulated frame nor the
    /// video settings changed since the last call, so a paused emulator
    /// doesn't keep resubmitting identical work.
    ///
    /// `display_px` is one NDS screen's actual on-screen size in physical
    /// pixels (i.e. the layout rect's side length times
    /// `ctx.pixels_per_point()`), used to cap the factor actually computed —
    /// see [`effective_factor`].
    #[expect(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        ctx: &Context,
        nds: &NDS,
        options: TextureOptions,
        method: UpscaleMethod,
        factor: u8,
        display_px: f32,
        dirty: bool,
    ) {
        if dirty {
            let max_texture_side = ctx.input(|i| i.max_texture_side);
            let factor = effective_factor(factor, display_px, max_texture_side);
            self.last_effective_factor = factor;

            let [top_pixels, bottom_pixels] = nds.get_screens();
            self.top_worker.submit(ScreenJob { pixels: top_pixels.clone(), method, factor });
            self.bottom_worker.submit(ScreenJob { pixels: bottom_pixels.clone(), method, factor });
        }

        if let Some(image) = self.top_worker.try_take_result() {
            self.top.set(image, options);
        }
        if let Some(image) = self.bottom_worker.try_take_result() {
            self.bottom.set(image, options);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factor_clamps_to_backend_texture_limit() {
        assert_eq!(clamp_factor_to_texture_limit(16, 8192), 16);
        // 256 * 16 = 4096 > 2048, so factor must be reduced.
        assert_eq!(clamp_factor_to_texture_limit(16, 2048), 8);
        assert_eq!(clamp_factor_to_texture_limit(2, 2048), 2);
    }

    #[test]
    fn effective_factor_caps_to_display_size() {
        // An 800px-tall screen only needs ceil(800/256) = 4x, even though
        // the user asked for 16x.
        assert_eq!(effective_factor(16, 800.0, 8192), 4);
        // Never exceeds the nominal factor even if the display is huge.
        assert_eq!(effective_factor(4, 8000.0, 8192), 4);
        // Small displays still get at least MIN_FACTOR.
        assert_eq!(effective_factor(16, 1.0, 8192), 1);
        // Non-finite/garbage display size never panics or produces 0.
        assert_eq!(effective_factor(16, f32::NAN, 8192), 1);
        assert_eq!(effective_factor(16, -5.0, 8192), 1);
    }
}
