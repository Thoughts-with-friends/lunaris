//! Running the console: pacing it, feeding it, and reporting when it stops.

use super::*;

impl MelonEgui {
    /// Run however many emulated frames wall-clock time has earned, then upload
    /// the resulting picture.
    pub(crate) fn advance(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let elapsed = self.last_tick.elapsed();
        self.last_tick = Instant::now();
        self.poll_lan();
        self.poll_remote();
        // Before the early return: a dialog may be the only thing that will
        // produce a cart to run.
        self.poll_dialog();

        // A client has no console to advance: its picture arrives over the
        // network and its input goes back the same way. Everything below this
        // point is about running an emulator, which is precisely what a client
        // does not do.
        if self.mode == Mode::RemoteClient {
            self.service_remote_client(ctx);
            self.report_fps();
            return;
        }

        // Pumped before the early return: gilrs only notices a pad being
        // plugged in while its queue is drained, and the Input pane lists
        // controllers whether or not a cart is running.
        let pad = self.pads.poll(&self.bindings);
        // Each click of the left stick steps to the next speed, which is why
        // this is a count and not a flag -- see `crate::pad::PadSample`.
        for _ in 0..pad.speed_clicks {
            self.cycle_speed();
        }
        let pad_keys = pad.keys;

        if self.emu.is_none() {
            return;
        }

        // Real time unless the speed control says otherwise, and always real
        // time when a second console is listening -- see
        // [`Self::effective_speed`].
        let speed = self.effective_speed();
        // The catch-up ceiling has to rise with the speed or it becomes the
        // speed: at 4x a repaint legitimately owes four times as many frames,
        // and a cap below that both truncates the burst and trips the
        // debt-dropping guard below on ordinary jitter.
        let catch_up = catch_up_limit(speed);

        // melonDS's "pause when unfocused": the console keeps its state, it just
        // stops advancing while the window is in the background.
        let unfocused = self.pause_when_unfocused && !ctx.input(|i| i.focused);
        let due = if self.paused || unfocused {
            // Debt accrued while paused is not owed: resuming should not
            // fast-forward through it.
            self.frame_debt = 0.0;
            u32::from(std::mem::take(&mut self.step_pending))
        } else if let Some((at, _)) = &self.shot {
            // A capture has to land on an exact frame to be worth comparing
            // against another run, so the burst is cut short at the target and
            // the console stops there — it must not keep running while the
            // screenshot makes its way back from the GPU.
            UNLIMITED_BURST.min(at.saturating_sub(self.frames_run) as u32)
        } else if !self.limit_framerate {
            UNLIMITED_BURST
        } else {
            // Ordinarily the DS's own rate. On a LAN link it is instead what
            // the link can sustain: `mp_recv_replies` blocks inside a frame for
            // as long as a reply takes to come back, so a console on a slow
            // link genuinely cannot issue 59.83 frames a second. Pacing to the
            // native rate anyway only builds a debt that is discharged as a
            // burst of rounds, which floods the peer — the very thing that
            // turns a slow link into a broken one.
            let rate = self.lan_pace.as_ref().map_or(FRAME_RATE, crate::lan::LinkPace::frame_rate)
                * f64::from(speed);
            earn_frames(&mut self.frame_debt, elapsed.as_secs_f64(), rate, catch_up)
        };

        // "Audio sync": when the ring is already comfortably full the sound
        // card has not caught up, so this repaint runs nothing and lets it. Only
        // meaningful while the framerate limiter is on and the console is
        // running at real time -- both fast-forward and the speed control
        // deliberately outrun the device, and at 2x the ring is *always* past
        // the mark, so leaving this in would clamp every repaint to zero frames
        // and the speed setting would do nothing at all.
        let due = if self.audio_sync
            && self.limit_framerate
            && !self.paused
            && crate::speed::is_real_time(speed)
        {
            match &self.audio {
                Ok(audio) if audio.fill() > 0.75 => 0,
                _ => due,
            }
        } else {
            due
        };

        // Sampled before the core is borrowed, since both readings come out of
        // `self` and the emulator borrow would conflict with them.
        self.poll_rebind(ctx);
        // A key pressed while the Input dialog is waiting for one is a
        // rebinding, not a button press: handing it to the cart as well would
        // make the console jump every time somebody changed a binding.
        let keys = if self.listening.is_some() {
            0
        } else {
            ctx.input(|i| self.bindings.key_mask(i)) | pad_keys
        };
        let touch = self.sample_touch(ctx);
        // The guest window has its own input; see `guest_view`.
        let (guest_keys, guest_touch) = self.sample_guest_input(ctx);

        let mut ran = 0;
        let mut stopped = false;
        // The core's account of why, which only exists for as long as it takes
        // to read it out of the console that gave it.
        let mut stop_note = None;
        self.apply_render_settings();
        self.apply_renderer();
        self.apply_cheats();
        let mic_static = self.mic_static;

        // The second console runs on its own thread; all that reaches it from
        // here is the input its window collected.
        if let Some(guest) = &self.guest {
            guest.set_input(guest_keys, guest_touch);
            guest.set_paused(self.paused || unfocused);
        }

        let Self { emu, .. } = self;

        if let Some(emu) = emu.as_mut() {
            emu.nds.set_keys(keys);
            emu.set_mic_static(mic_static);
            match touch {
                Some((x, y)) => emu.nds.touch(x, y),
                None => emu.nds.release_screen(),
            }
        }
        //
        // Whether a console is still running is *asked*, not inferred from the
        // frame's scanline count. A console asleep — which is a state a cart
        // enters deliberately, and which the wireless code passes through —
        // draws no scanlines at all, and reading that as "stopped" is what used
        // to freeze a perfectly healthy console the moment it slept.
        for _ in 0..due {
            if let Some(emu) = emu.as_mut() {
                emu.nds.run_frame();
                if !emu.nds.is_running() {
                    stop_note = emu.stop_reason();
                    stopped = true;
                    break;
                }
                ran += 1;
            }
        }

        // Whatever the second console's thread has to say. Its own stop is
        // reported the same way the first console's is, but it is closed rather
        // than paused: a console that has stopped is not going to draw again.
        if let Some(note) = self.guest.as_ref().and_then(crate::guest::Guest::take_note) {
            self.post_warn(format!("second instance {note}"));
            if self.guest.as_ref().is_some_and(crate::guest::Guest::finished) {
                self.write_crash_report("second instance", &note);
            }
        }
        if self.guest.as_ref().is_some_and(crate::guest::Guest::finished) {
            self.guest = None;
            self.guest_textures = None;
        }
        if stopped {
            let note = stop_note.unwrap_or_else(|| "stopped".to_owned());
            self.post_error(format!("console {note}"));
            self.write_crash_report("console 0", &note);
        }

        if ran > 0 {
            self.drain_audio();
        }
        self.fps_frames += ran;
        self.frames_run += u64::from(ran);
        if stopped {
            self.paused = true;
            self.post_error("core stopped");
        }

        if ran > 0 {
            self.upload(ctx, frame);
        }

        let window = self.fps_since.elapsed();
        if window >= Duration::from_secs(1) {
            self.fps = f64::from(self.fps_frames) / window.as_secs_f64();
            self.fps_frames = 0;
            self.fps_since = Instant::now();
        }

        if self.last_save_flush.elapsed() >= SAVE_FLUSH_INTERVAL {
            self.last_save_flush = Instant::now();
            if let Some(emu) = &self.emu {
                emu.flush_save();
            }
            // The second console keeps its own `.sav` under `instance2/saves`,
            // and a window closed by the task manager would otherwise lose
            // whatever it had not written.
            if let Some(guest) = &self.guest {
                guest.send(crate::guest::Command::FlushSave);
            }
        }
    }

    /// Push the Video settings' renderer half into the core.
    ///
    /// Only on a change: swapping renderer reallocates every render target,
    /// and even a change of internal resolution is not free.
    pub(crate) fn apply_renderer(&mut self) {
        use crate::video::Renderer;

        // Asking for OpenGL without a working blitter would leave a console
        // rendering into a texture nothing can draw, so that choice is taken
        // back here rather than in the core.
        if self.video.renderer.is_gl() && !self.gl_available() {
            self.video.renderer = Renderer::Software;
        }
        let wanted = self.video.to_core();
        if self.applied_renderer == Some(wanted) || self.emu.is_none() {
            return;
        }

        let Some(emu) = &mut self.emu else { return };
        let installed = Renderer::from_core(emu.set_render_settings(wanted));
        // The compute renderer builds its shaders on demand; without this the
        // first frames come out empty.
        let mut compiled = 0;
        while emu.gl_shader_compile_step().is_some() && compiled < SHADER_COMPILE_LIMIT {
            compiled += 1;
        }

        self.applied_renderer = Some(wanted);
        if installed.is_gl() {
            // The CPU textures stop being refreshed under OpenGL and would
            // otherwise linger as a stale picture.
            self.textures = None;
        }
        if installed == self.video.renderer {
            self.post(match installed {
                Renderer::Software => format!(
                    "renderer: software{}",
                    if self.video.threaded_software { ", threaded" } else { "" }
                ),
                _ => format!(
                    "renderer: {} at {}x internal resolution",
                    installed.label(),
                    self.video.scale()
                ),
            });
        } else {
            // melonDS installs a software renderer of its own when the one it
            // was handed cannot initialise, so this is what actually happened
            // rather than what was asked for.
            self.post_warn(format!(
                "could not create the {} renderer; on {} instead",
                self.video.renderer.label(),
                installed.label()
            ));
            self.video.renderer = installed;
            self.applied_renderer = Some(self.video.to_core());
        }
    }

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
            emu.set_render(wanted.0);
            emu.set_displayed_screens(wanted.1);
            self.applied_render = Some(wanted);
        }
    }

    /// Move whatever the SPU has produced into the output ring.
    ///
    /// Called once per batch of emulated frames rather than per frame, since the
    /// core buffers internally and the ring is what actually paces playback.
    pub(crate) fn drain_audio(&mut self) {
        let (Ok(audio), Some(emu)) = (&mut self.audio, &mut self.emu) else {
            return;
        };
        let queued = emu.nds.audio_queued();
        if queued == 0 {
            return;
        }
        // Interleaved stereo, so two `i16` per sample frame.
        let mut buf = vec![0i16; queued * 2];
        let read = emu.nds.read_audio(&mut buf);
        audio.push(&buf[..read * 2]);
    }
}

/// How many emulated frames `elapsed` seconds at `rate` frames a second have
/// earned, up to `cap`, carrying the fraction in `debt`.
///
/// Split out of [`MelonEgui::advance`] so the pacing can be driven from a test
/// without a console: this is the whole of what the speed setting changes, and
/// "does 2x actually run twice as many frames" is a question about this
/// function alone.
///
/// Whatever is still owed past `cap` is dropped rather than carried, so a stall
/// -- a dragged window, a paused debugger -- cannot turn into a burst later.
pub(crate) fn earn_frames(debt: &mut f64, elapsed: f64, rate: f64, cap: u32) -> u32 {
    *debt += elapsed * rate;
    let due = (*debt as u32).min(cap);
    *debt -= f64::from(due);
    if *debt > f64::from(cap) {
        *debt = 0.0;
    }
    due
}

/// How many emulated frames one repaint may run to catch up, at `speed`.
///
/// [`MAX_CATCH_UP`] is the real-time budget; running at *n* times real time
/// needs *n* times as much of it, and the ceiling is rounded up so a fractional
/// speed is not quietly capped below itself.
pub(crate) fn catch_up_limit(speed: f32) -> u32 {
    let scale = speed.ceil().max(1.0) as u32;
    MAX_CATCH_UP.saturating_mul(scale)
}

impl MelonEgui {
    /// The speed the console is actually run at, which is the speed that was
    /// asked for except where two consoles have to agree about time.
    ///
    /// A second console runs on its own thread at its own rate and a LAN peer
    /// runs on another machine entirely; neither is told about this setting, so
    /// running the local console faster would desynchronise a linked game
    /// outright. The LAN case is worse than a desync -- `lan_pace` exists
    /// precisely because issuing rounds faster than the link sustains floods
    /// the peer.
    #[must_use]
    pub fn effective_speed(&self) -> f32 {
        if self.lan_pace.is_some() || self.guest.is_some() {
            crate::speed::DEFAULT
        } else {
            crate::speed::clamp(self.speed)
        }
    }

    /// Whether [`Self::effective_speed`] is being held at real time rather than
    /// following the setting, so the UI can say why the control is inert.
    #[must_use]
    pub fn speed_locked(&self) -> bool {
        self.lan_pace.is_some() || self.guest.is_some()
    }

    /// Step to the next speed, announcing it: this is bound to a pad button
    /// with no label on it, so the only feedback is what is said here.
    pub fn cycle_speed(&mut self) {
        self.speed = crate::speed::next(self.speed);
        // The debt was accrued at the old rate; carrying it over would spend it
        // at the new one, which is a lurch in whichever direction the speed
        // changed.
        self.frame_debt = 0.0;
        if self.speed_locked() {
            self.post_warn(format!(
                "speed {} (not applied: a second console is running)",
                crate::speed::label(self.speed)
            ));
        } else {
            self.post(format!("speed {}", crate::speed::label(self.speed)));
        }
    }

    /// Set the speed from the UI, clamped to the offered range.
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = crate::speed::clamp(speed);
        self.frame_debt = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::{FRAME_RATE, MAX_CATCH_UP, catch_up_limit, earn_frames};

    /// One second of 60 Hz repaints at `speed`, as the pacing loop would run
    /// them: how many emulated frames come out.
    fn frames_in_one_second(speed: f32) -> u32 {
        let mut debt = 0.0;
        let cap = catch_up_limit(speed);
        let rate = FRAME_RATE * f64::from(speed);
        (0..60).map(|_| earn_frames(&mut debt, 1.0 / 60.0, rate, cap)).sum()
    }

    #[test]
    fn one_second_of_repaints_earns_the_speed_it_was_asked_for() {
        // The DS's own rate, to within the frame the debt is still carrying.
        assert!((59..=60).contains(&frames_in_one_second(1.0)), "{}", frames_in_one_second(1.0));
        assert!((29..=30).contains(&frames_in_one_second(0.5)), "{}", frames_in_one_second(0.5));
        assert!((119..=120).contains(&frames_in_one_second(2.0)), "{}", frames_in_one_second(2.0));
        assert!((239..=240).contains(&frames_in_one_second(4.0)), "{}", frames_in_one_second(4.0));
    }

    #[test]
    fn a_stall_is_dropped_rather_than_paid_back_as_a_burst() {
        let mut debt = 0.0;
        // Ten seconds of nothing: nearly 600 frames owed, capped at four.
        let due = earn_frames(&mut debt, 10.0, FRAME_RATE, MAX_CATCH_UP);
        assert_eq!(due, MAX_CATCH_UP);
        assert_eq!(debt, 0.0, "the surplus is dropped, not carried");
    }

    #[test]
    fn the_catch_up_budget_grows_with_the_speed() {
        assert_eq!(catch_up_limit(1.0), MAX_CATCH_UP);
        assert_eq!(
            catch_up_limit(0.5),
            MAX_CATCH_UP,
            "a slow speed still gets the real-time budget"
        );
        assert_eq!(catch_up_limit(2.0), MAX_CATCH_UP * 2);
        assert_eq!(catch_up_limit(4.0), MAX_CATCH_UP * 4);
        // 1.5x owes more than one frame per repaint, so it needs more than the
        // 1x budget or the cap would hold it below its own speed.
        assert!(catch_up_limit(1.5) > MAX_CATCH_UP);
    }
}
