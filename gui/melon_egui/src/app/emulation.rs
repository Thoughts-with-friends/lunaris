//! Running the console: pacing it, feeding it, and reporting when it stops.

use super::*;

impl MelonEgui {
    /// Gather everything that might explain a stopped console, show it, and
    /// write it beside the executable.
    ///
    /// Written to a file because the usual way to run this is by launching the
    /// executable, which on Windows has no console attached: a diagnostic that
    /// only reaches stderr reaches nobody. The pane opens by itself for the
    /// same reason.
    pub(crate) fn write_crash_report(&mut self, who: &str, note: &str) {
        let mut report = format!("melon_egui: {who} {note}\n");
        if let Some(emu) = &mut self.emu {
            report.push_str(&format!(
                "cart: {} [{}]\nframes run: console 0 = {}",
                emu.info.title,
                emu.info.gamecode,
                emu.nds.frame_count()
            ));
        }
        if let Some(guest) = &self.guest {
            report.push_str(&format!(", second instance = {}", guest.frame_count()));
        }
        report.push('\n');

        // Who was on the air, and what they had exchanged: local play failing
        // shows up here as one side sending and nothing coming back.
        let connected = self.airwaves.connected();
        for (i, counters) in self.airwaves.counters().iter().enumerate().take(2) {
            report.push_str(&format!(
                "console {i}: {} | sent {}/{} cmd/reply, generic {}, ack {} | \
                 received cmd {}, reply {}, generic {} | stale replies {} | \
                 wifi clock {} | last reply mask {:04X}\n",
                if connected.get(i) == Some(&true) { "on the air" } else { "not on the air" },
                counters.sent_cmd,
                counters.sent_reply,
                counters.sent_generic,
                counters.sent_ack,
                counters.recv_cmd,
                counters.recv_reply,
                counters.recv_generic,
                counters.stale_replies,
                counters.clock,
                counters.last_reply_mask,
            ));
        }

        report.push_str("\n-- the last of the wireless traffic ------------------\n");
        let log = self.airwaves.log();
        for event in log.iter().rev().take(40).rev() {
            report.push_str(&format!(
                "console {} {} len={} ts={}\n",
                event.sender,
                event.kind.label(),
                event.len,
                event.timestamp
            ));
        }

        report.push_str("\n-- the core's own last words -------------------------\n");
        for line in crate::logger::recent() {
            report.push_str(&line);
            report.push('\n');
        }

        let path = crate::config::config_dir().join("last-stop.txt");
        match std::fs::create_dir_all(crate::config::config_dir())
            .and_then(|()| std::fs::write(&path, &report))
        {
            Ok(()) => {
                eprintln!("melon_egui: wrote {}", path.display());
                self.post(format!("stop report written to {}", path.display()));
            }
            Err(e) => eprintln!("melon_egui: could not write {}: {e}", path.display()),
        }
        self.crash_report = Some(report);
        if !self.panes.contains(&panes::Pane::Crash) {
            self.panes.push(panes::Pane::Crash);
        }
    }

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
        let pad_keys = self.pads.poll(&self.bindings);

        if self.emu.is_none() {
            return;
        }

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
            let rate = self.lan_pace.as_ref().map_or(FRAME_RATE, crate::lan::LinkPace::frame_rate);
            self.frame_debt += elapsed.as_secs_f64() * rate;
            let due = (self.frame_debt as u32).min(MAX_CATCH_UP);
            self.frame_debt -= f64::from(due);
            // Whatever is still owed after the cap is dropped rather than
            // carried, so a stall cannot turn into a burst later.
            if self.frame_debt > f64::from(MAX_CATCH_UP) {
                self.frame_debt = 0.0;
            }
            due
        };

        // "Audio sync": when the ring is already comfortably full the sound
        // card has not caught up, so this repaint runs nothing and lets it. Only
        // meaningful while the framerate limiter is on -- fast-forward
        // deliberately outruns the device.
        let due = if self.audio_sync && self.limit_framerate && !self.paused {
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
            self.post(format!("second instance {note}"));
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
            eprintln!("melon_egui: console {note}");
            self.post(format!("console {note}"));
            self.write_crash_report("console 0", &note);
        }

        if ran > 0 {
            self.drain_audio();
        }
        self.fps_frames += ran;
        self.frames_run += u64::from(ran);
        if stopped {
            self.paused = true;
            self.post("core stopped");
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
            self.post(format!(
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
