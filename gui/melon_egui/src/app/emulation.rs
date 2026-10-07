//! Running the first console once per repaint.
//!
//! # One repaint of [`MelonEgui::advance`]
//!
//! ```text
//!  1. poll_background   finished LAN / Remote Desktop connections, file dialogs
//!  2. (client only)     show the host's picture, send our input → return
//!  3. poll pads         gamepad state, speed-button clicks
//!  4. frames_due        how many frames wall-clock time has earned
//!                       (0 while paused, capped burst for --shot / unlimited)
//!  5. sample_input      keyboard + pad keys, stylus, the second window's input
//!  6. apply_core_settings  renderer, screen mask, cheats — only on a change
//!  7. run_frames        run the frames; stop early if the console stops
//!  8. after_frames      second console's notes, audio, picture upload
//!  9. bookkeeping       FPS readout (1 s window), save flush (1 s)
//! ```
//!
//! The pacing itself is two pure functions at the bottom ([`earn_frames`],
//! [`catch_up_limit`]) so it can be tested without a console.

use super::*;

impl MelonEgui {
    /// Run however many emulated frames wall-clock time has earned, then upload
    /// the resulting picture. Called once per repaint from `update`.
    pub(crate) fn advance(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let elapsed = self.last_tick.elapsed();
        self.last_tick = Instant::now();
        // Before the early returns: a dialog may be what produces a cart.
        self.poll_background();

        if self.mode == Mode::RemoteClient {
            self.service_remote_client(ctx);
            self.report_fps(Duration::from_millis(500));
            return;
        }

        // Pumped even without a cart: gilrs only notices a pad being plugged
        // in while its queue is drained, and the Input pane lists pads anyway.
        let pad = self.pads.poll(&self.bindings);
        for _ in 0..pad.speed_clicks {
            self.cycle_speed();
        }
        if self.emu.is_none() {
            return;
        }

        // melonDS's "pause when unfocused": state kept, frames not advanced.
        let unfocused = self.pause_when_unfocused && !ctx.input(|i| i.focused);
        let due = self.frames_due(elapsed, unfocused);
        let (keys, touch) = self.sample_input(ctx, pad.keys);
        let (guest_keys, guest_touch) = self.sample_guest_input(ctx);
        self.apply_core_settings();

        // The second console runs on its own thread; it only gets its input.
        if let Some(guest) = &self.guest {
            guest.set_input(guest_keys, guest_touch);
            guest.set_paused(self.paused || unfocused);
        }

        let (ran, stopped) = self.run_frames(due, keys, touch);
        self.after_frames(ctx, frame, ran, stopped);
        self.report_fps(Duration::from_secs(1));
        self.flush_saves_periodically();
    }

    /// Collect whatever finished off the UI thread since the last repaint.
    fn poll_background(&mut self) {
        self.poll_lan();
        self.poll_remote();
        self.poll_dialog();
    }

    /// How many frames to run this repaint.
    ///
    /// * paused / unfocused — none, except one owed by Frame step;
    /// * `--shot` pending — a burst that stops exactly on the target frame;
    /// * framerate limiter off — a fixed burst;
    /// * otherwise — what the elapsed time earned at the DS rate × speed (or
    ///   at the rate a LAN link can sustain), minus audio-sync throttling.
    fn frames_due(&mut self, elapsed: Duration, unfocused: bool) -> u32 {
        let speed = self.effective_speed();
        let due = if self.paused || unfocused {
            // Time spent paused is not owed.
            self.frame_debt = 0.0;
            u32::from(std::mem::take(&mut self.step_pending))
        } else if let Some((at, _)) = &self.shot {
            UNLIMITED_BURST.min(at.saturating_sub(self.frames_run) as u32)
        } else if !self.limit_framerate {
            UNLIMITED_BURST
        } else {
            // On a LAN link a round blocks inside the frame, so pacing to the
            // native rate would only build a debt paid back as a burst that
            // floods the peer. See `crate::lan::LinkPace`.
            let rate = self.lan_pace.as_ref().map_or(FRAME_RATE, crate::lan::LinkPace::frame_rate)
                * f64::from(speed);
            earn_frames(&mut self.frame_debt, elapsed.as_secs_f64(), rate, catch_up_limit(speed))
        };

        // Audio sync: a ring more than 3/4 full means the sound card is behind,
        // so this repaint runs nothing. Only at real time with the limiter on —
        // any other speed outruns the card on purpose.
        let audio_sync = self.audio_sync
            && self.limit_framerate
            && !self.paused
            && crate::speed::is_real_time(speed);
        match &self.audio {
            Ok(audio) if audio_sync && audio.fill() > 0.75 => 0,
            _ => due,
        }
    }

    /// This repaint's buttons and stylus for the first console.
    ///
    /// While the Input dialog waits for a key, the press becomes a binding
    /// (`poll_rebind`) and the console sees no buttons at all.
    fn sample_input(&mut self, ctx: &egui::Context, pad_keys: u32) -> (u32, Option<(u16, u16)>) {
        self.poll_rebind(ctx);
        (self.held_keys(ctx, pad_keys), self.sample_touch(ctx))
    }

    /// The keyboard's DS key mask merged with the pads', or 0 while rebinding.
    pub(crate) fn held_keys(&self, ctx: &egui::Context, pad_keys: u32) -> u32 {
        if self.listening.is_some() {
            0
        } else {
            ctx.input(|i| self.bindings.key_mask(i)) | pad_keys
        }
    }

    /// Push every setting the core holds a copy of, each only when it changed.
    fn apply_core_settings(&mut self) {
        self.apply_render_settings();
        self.apply_renderer();
        self.apply_cheats();
    }

    /// Run `due` frames on the first console with this input.
    ///
    /// Returns how many ran and whether the console stopped (in which case the
    /// stop is reported and a crash report written).
    fn run_frames(&mut self, due: u32, keys: u32, touch: Option<(u16, u16)>) -> (u32, bool) {
        let Some(emu) = self.emu.as_mut() else { return (0, false) };
        emu.set_input(keys, touch);
        emu.nds.set_mic_static(self.mic_static);

        let mut ran = 0;
        let mut stop_note = None;
        for _ in 0..due {
            if let Err(note) = emu.run_frame_checked() {
                stop_note = Some(note);
                break;
            }
            ran += 1;
        }

        self.collect_guest_notes();
        let stopped = stop_note.is_some();
        if let Some(note) = stop_note {
            self.post_error(self.i18n().f(K::ConsoleNote, &[&note]));
            self.write_crash_report("console 0", &note);
        }
        (ran, stopped)
    }

    /// Report what the second console's thread said, and close it once it has
    /// stopped for good.
    fn collect_guest_notes(&mut self) {
        let finished = |app: &Self| app.guest.as_ref().is_some_and(crate::guest::Guest::finished);
        if let Some(note) = self.guest.as_ref().and_then(crate::guest::Guest::take_note) {
            self.post_warn(self.i18n().f(K::SecondConsoleNote, &[&note.render(self.i18n())]));
            if finished(self) {
                // The report is a diagnostic file, kept in English.
                self.write_crash_report("second instance", &note.english());
            }
        }
        if finished(self) {
            self.close_guest();
        }
    }

    /// Audio, counters and the picture, once a batch of frames has run.
    fn after_frames(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        ran: u32,
        stopped: bool,
    ) {
        if ran > 0 {
            self.drain_audio();
        }
        self.fps_frames += ran;
        self.frames_run += u64::from(ran);
        if stopped {
            self.paused = true;
            self.post_error(self.i18n().s(K::CoreStopped));
        }
        if ran > 0 {
            self.upload(ctx, frame);
        }
    }

    /// Write pending backup memory once a second, for both consoles, so a
    /// window killed by the task manager loses at most a second of saving.
    fn flush_saves_periodically(&mut self) {
        if self.last_save_flush.elapsed() < SAVE_FLUSH_INTERVAL {
            return;
        }
        self.last_save_flush = Instant::now();
        if let Some(emu) = &self.emu {
            emu.flush_save();
        }
        if let Some(guest) = &self.guest {
            guest.send(crate::guest::Command::FlushSave);
        }
    }

    /// Recompute the FPS readout once `window` has passed. A Remote Desktop
    /// client counts *received* frames the same way.
    pub(crate) fn report_fps(&mut self, window: Duration) {
        let elapsed = self.fps_since.elapsed();
        if elapsed >= window {
            self.fps = f64::from(self.fps_frames) / elapsed.as_secs_f64();
            self.fps_frames = 0;
            self.fps_since = Instant::now();
        }
    }

    /// Push the Video settings' renderer choice into the core, on a change only
    /// (a renderer swap reallocates every render target).
    ///
    /// melonDS falls back to software when the asked-for renderer cannot start;
    /// that is reported and the setting corrected to what actually happened.
    pub(crate) fn apply_renderer(&mut self) {
        use crate::video::Renderer;

        // OpenGL without a working blitter would draw into a texture nothing
        // can show.
        if self.video.renderer.is_gl() && !self.gl_available() {
            self.video.renderer = Renderer::Software;
        }
        let wanted = self.video.to_core();
        if self.applied_renderer == Some(wanted) {
            return;
        }
        let Some(emu) = &mut self.emu else { return };
        let installed = Renderer::from_core(emu.nds.set_render_settings(wanted));
        // The compute renderer builds its shaders on demand; without this the
        // first frames come out empty. Bounded, so a driver that never reports
        // done cannot hang the window.
        let mut compiled = 0;
        while emu.nds.gl_shader_compile_step().is_some() && compiled < SHADER_COMPILE_LIMIT {
            compiled += 1;
        }

        self.applied_renderer = Some(wanted);
        if installed.is_gl() {
            // The CPU textures stop being refreshed under OpenGL.
            self.textures = None;
        }
        let tr = self.i18n();
        if installed == self.video.renderer {
            self.post(match installed {
                Renderer::Software if self.video.threaded_software => {
                    tr.s(K::RendererNowSoftwareThreaded)
                }
                Renderer::Software => tr.s(K::RendererNowSoftware),
                _ => tr.f(K::RendererNowGl, &[&installed.label(tr), &self.video.scale()]),
            });
        } else {
            self.post_warn(
                tr.f(K::RendererFellBack, &[&self.video.renderer.label(tr), &installed.label(tr)]),
            );
            self.video.renderer = installed;
            self.applied_renderer = Some(self.video.to_core());
        }
    }

    /// Push "Render frames" and the displayed-screens mask, on a change only.
    ///
    /// The mask follows the *explicit* single-screen sizings only: under
    /// `Auto`, hiding a screen would stop it being drawn, and the stale screen
    /// would then be read as idle — a feedback loop.
    pub(crate) fn apply_render_settings(&mut self) {
        let (top, bottom) = match self.view.sizing {
            ScreenSizing::TopOnly => (!self.view.swap, self.view.swap),
            ScreenSizing::BottomOnly => (self.view.swap, !self.view.swap),
            _ => (true, true),
        };
        let wanted = (self.video.render, self.video.displayed_mask(top, bottom));
        if self.applied_render == Some(wanted) {
            return;
        }
        if let Some(emu) = &mut self.emu {
            emu.nds.set_render(wanted.0);
            emu.nds.set_displayed_screens(wanted.1);
            self.applied_render = Some(wanted);
        }
    }

    /// Move whatever the SPU produced into the output ring.
    pub(crate) fn drain_audio(&mut self) {
        let (Ok(audio), Some(emu)) = (&mut self.audio, &mut self.emu) else {
            return;
        };
        let samples = emu.drain_audio(usize::MAX);
        if !samples.is_empty() {
            audio.push(&samples);
        }
    }

    /// The speed actually used: the setting, except real time whenever a
    /// second console or a LAN peer has to agree with this one about time.
    #[must_use]
    pub fn effective_speed(&self) -> f32 {
        if self.speed_locked() { crate::speed::DEFAULT } else { crate::speed::clamp(self.speed) }
    }

    /// Whether the speed is held at real time (so the UI can say why).
    #[must_use]
    pub fn speed_locked(&self) -> bool {
        self.lan_pace.is_some() || self.guest.is_some()
    }

    /// Step to the next speed (the pad's left-stick click) and announce it.
    pub fn cycle_speed(&mut self) {
        self.speed = crate::speed::next(self.speed);
        // Debt earned at the old rate would lurch at the new one.
        self.frame_debt = 0.0;
        if self.speed_locked() {
            self.post_warn(self.i18n().f(K::SpeedLocked, &[&crate::speed::label(self.speed)]));
        } else {
            self.post(self.i18n().f(K::SpeedNow, &[&crate::speed::label(self.speed)]));
        }
    }

    /// Set the speed from the UI, clamped to the offered range.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = crate::speed::clamp(speed);
        self.frame_debt = 0.0;
    }
}

/// How many frames `elapsed` seconds at `rate` frames a second have earned, up
/// to `cap`, carrying the fraction in `debt`.
///
/// What is still owed past `cap` is dropped, so a stall (a dragged window, a
/// debugger) never turns into a burst later.
pub(crate) fn earn_frames(debt: &mut f64, elapsed: f64, rate: f64, cap: u32) -> u32 {
    *debt += elapsed * rate;
    let due = (*debt as u32).min(cap);
    *debt -= f64::from(due);
    if *debt > f64::from(cap) {
        *debt = 0.0;
    }
    due
}

/// The most frames one repaint may run at `speed`: [`MAX_CATCH_UP`] times the
/// speed rounded up, so 4x is not capped below 4x.
pub(crate) fn catch_up_limit(speed: f32) -> u32 {
    let scale = speed.ceil().max(1.0) as u32;
    MAX_CATCH_UP.saturating_mul(scale)
}

#[cfg(test)]
mod tests {
    use super::{FRAME_RATE, MAX_CATCH_UP, catch_up_limit, earn_frames};

    /// One second of 60 Hz repaints at `speed`: how many frames come out.
    fn frames_in_one_second(speed: f32) -> u32 {
        let mut debt = 0.0;
        let cap = catch_up_limit(speed);
        let rate = FRAME_RATE * f64::from(speed);
        (0..60).map(|_| earn_frames(&mut debt, 1.0 / 60.0, rate, cap)).sum()
    }

    #[test]
    fn one_second_of_repaints_earns_the_speed_it_was_asked_for() {
        assert!((59..=60).contains(&frames_in_one_second(1.0)), "{}", frames_in_one_second(1.0));
        assert!((29..=30).contains(&frames_in_one_second(0.5)), "{}", frames_in_one_second(0.5));
        assert!((119..=120).contains(&frames_in_one_second(2.0)), "{}", frames_in_one_second(2.0));
        assert!((239..=240).contains(&frames_in_one_second(4.0)), "{}", frames_in_one_second(4.0));
    }

    #[test]
    fn a_stall_is_dropped_rather_than_paid_back_as_a_burst() {
        let mut debt = 0.0;
        let due = earn_frames(&mut debt, 10.0, FRAME_RATE, MAX_CATCH_UP);
        assert_eq!(due, MAX_CATCH_UP);
        assert_eq!(debt, 0.0, "the surplus is dropped, not carried");
    }

    #[test]
    fn the_catch_up_budget_grows_with_the_speed() {
        assert_eq!(catch_up_limit(1.0), MAX_CATCH_UP);
        assert_eq!(catch_up_limit(0.5), MAX_CATCH_UP, "a slow speed keeps the real-time budget");
        assert_eq!(catch_up_limit(2.0), MAX_CATCH_UP * 2);
        assert_eq!(catch_up_limit(4.0), MAX_CATCH_UP * 4);
        assert!(catch_up_limit(1.5) > MAX_CATCH_UP);
    }
}
