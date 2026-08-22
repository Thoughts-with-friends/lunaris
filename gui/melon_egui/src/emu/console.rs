//! A booted console, and everything done to one.

use super::*;

/// A booted cart, plus the host-side state that outlives any single frame.
pub struct Emu {
    pub nds: Nds,
    /// Where the ROM came from, for window titles and for deriving the save and
    /// savestate paths.
    pub rom_path: PathBuf,
    pub info: CartInfo,
    /// The other end of the [`SaveSink`] handed to the core, so the front end
    /// can flush pending backup memory on its own schedule.
    pub(crate) saves: Arc<SaveSink>,
    /// Where savestates go; `None` means beside the ROM.
    pub(crate) state_dir: Option<PathBuf>,
    /// The reason the core last gave for stopping, filled in from the host
    /// callback while `run_frame` was running.
    pub(crate) stop: Arc<Mutex<Option<StopReason>>>,
    /// This console's seat on the airwaves and the instance number that goes
    /// with it, kept so that a reboot takes the same seat rather than dropping
    /// off the air.
    pub(crate) seat: Option<(u32, crate::mp::Client)>,
}

impl Emu {
    /// Read `rom_path`, restore its backup memory if a `.sav` sits beside it,
    /// and direct-boot the cart.
    ///
    /// No BIOS or firmware files are needed: `melonds-sys`'s shim boots
    /// FreeBIOS with generated firmware.
    pub fn boot(rom_path: &Path) -> Result<Self, String> {
        Self::boot_with(rom_path, None, None)
    }

    /// As [`Emu::boot_with`], but joined to shared airwaves as console
    /// `instance_id`.
    ///
    /// `instance_id` also uniquifies the generated firmware's MAC address, the
    /// same way melonDS's frontend does, so two consoles on one medium are not
    /// indistinguishable to each other.
    pub fn boot_mp(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
        instance_id: u32,
        mp: crate::mp::Client,
    ) -> Result<Self, String> {
        Self::boot_inner(rom_path, save_dir, state_dir, instance_id, Some(mp), None)
    }

    /// Boot a cart using a LAN-backed melonDS multiplayer host.
    pub fn boot_lan(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
        transport: Box<dyn melonds::Host>,
    ) -> Result<Self, String> {
        Self::boot_inner(rom_path, save_dir, state_dir, 0, None, Some(transport))
    }

    /// As [`Emu::boot`], but with the save and savestate directories overridden;
    /// `None` for either means "beside the ROM".
    pub fn boot_with(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
    ) -> Result<Self, String> {
        Self::boot_inner(rom_path, save_dir, state_dir, 0, None, None)
    }

    pub(crate) fn boot_inner(
        rom_path: &Path,
        save_dir: Option<&PathBuf>,
        state_dir: Option<&PathBuf>,
        instance_id: u32,
        mp: Option<crate::mp::Client>,
        network: Option<Box<dyn melonds::Host>>,
    ) -> Result<Self, String> {
        let rom = std::fs::read(rom_path).map_err(|e| format!("cannot read ROM: {e}"))?;
        let save_path = crate::file::settings::Settings::redirect(save_dir, rom_path, "sav");
        let state_dir = state_dir.cloned();
        let save = std::fs::read(&save_path).ok();

        let saves = Arc::new(SaveSink { path: save_path, pending: Mutex::new(None) });
        let stop = Arc::new(Mutex::new(None));
        // The seat is cloned rather than moved: the console's `Host` owns one
        // handle to the airwaves, and the front end keeps another so a reboot
        // (importing a save) can take the same seat again.
        let seat = mp.clone().map(|mp| (instance_id, mp));
        let host = Box::new(HostBridge {
            saves: Arc::clone(&saves),
            stop: Arc::clone(&stop),
            mp,
            network,
        });

        let mut nds = Nds::new(&rom, save.as_deref(), instance_id, host)
            .map_err(|e| format!("cart rejected: {e}"))?;
        let (y, mo, d, h, mi, s) = if deterministic_rtc() { FIXED_RTC } else { utc_now() };
        nds.set_rtc(y, mo, d, h, mi, s);
        nds.boot();

        Ok(Self {
            nds,
            rom_path: rom_path.to_owned(),
            info: CartInfo::parse(&rom),
            saves,
            state_dir,
            stop,
            seat,
        })
    }

    /// Write out backup memory if the core has changed it since the last call.
    ///
    /// The core reports every write as it happens, which is far more often than
    /// a file should be rewritten, so [`SaveSink`] only remembers the newest
    /// image and the front end drains it on a timer.
    pub fn flush_save(&self) {
        let Some(data) = self.saves.pending.lock().unwrap().take() else {
            return;
        };
        if let Err(e) = std::fs::write(&self.saves.path, &data) {
            log::error!("failed to write {}: {e}", self.saves.path.display());
        }
    }

    /// Why the core stopped, if it has, taken out on the way past.
    ///
    /// Reported with both CPUs' program counters, since the reason alone does
    /// not say *where*: an ARM9 crash during wireless play, for instance, is
    /// usually a fault the ARM7's wifi handling led it into.
    pub fn stop_reason(&mut self) -> Option<String> {
        let reason = self.stop.lock().unwrap().take()?;
        Some(format!(
            "{} (ARM9 pc={:08X}, ARM7 pc={:08X})",
            reason.label(),
            self.nds.pc(),
            self.nds.arm7_pc()
        ))
    }

    /// Re-set the console's real-time clock.
    ///
    /// The RTC keeps counting in emulated time from whatever it is set to, so
    /// this takes effect immediately; carts that only read the clock at startup
    /// will not notice until they next look.
    pub fn set_clock(&mut self, clock: Clock) {
        self.nds.set_rtc(
            clock.year,
            clock.month,
            clock.day,
            clock.hour,
            clock.minute,
            clock.second,
        );
    }

    /// Apply the Video settings, returning the renderer the core actually
    /// installed.
    ///
    /// That can differ from the one asked for: melonDS falls back to the
    /// software renderer rather than leave a console unable to draw, so a
    /// machine whose driver cannot compile its shaders answers
    /// [`melonds::Renderer::Software`] here. Asking for an OpenGL renderer
    /// requires a current GL context on this thread, both here and on every
    /// subsequent frame.
    pub fn set_render_settings(&mut self, settings: melonds::RenderSettings) -> melonds::Renderer {
        self.nds.set_render_settings(settings)
    }

    /// Where the OpenGL renderer left the picture, if it is the one in use.
    pub fn gl_output(&mut self) -> Option<melonds::GlOutput> {
        self.nds.gl_output()
    }

    /// Build one of a lazily-compiled renderer's shaders, reporting progress
    /// as `(done, total)` while any remain. Only the compute renderer has
    /// any; for the others this is `None` from the first call.
    pub fn gl_shader_compile_step(&mut self) -> Option<(u32, u32)> {
        self.nds.gl_shader_compile_step()
    }

    /// Read one screen of the OpenGL renderer's output back into host memory,
    /// BGRA8888 and top-down, at the internal resolution.
    ///
    /// The headless harnesses capture the software renderer's framebuffers
    /// directly; this is how they capture a picture that only ever existed in
    /// a texture.
    pub fn gl_read_output(&mut self, screen: u8, out: &mut [u32]) -> usize {
        self.nds.gl_read_output(screen, out)
    }

    /// Open or close the lid, as melonDS's Power management does. Closing it
    /// raises the lid IRQ, which is how a cart is told to sleep.
    pub fn set_lid_closed(&mut self, closed: bool) {
        self.nds.set_lid_closed(closed);
    }

    pub fn lid_closed(&mut self) -> bool {
        self.nds.lid_closed()
    }

    /// What the power-management chip reports about the battery: `true` for
    /// okay, `false` for the low level a cart warns about.
    pub fn set_battery_okay(&mut self, okay: bool) {
        self.nds.set_battery_okay(okay);
    }

    pub fn battery_okay(&mut self) -> bool {
        self.nds.battery_okay()
    }

    /// Turn framebuffer production on or off.
    ///
    /// Off, the console keeps running and keeps capturing to VRAM; only the
    /// framebuffer the front end reads goes stale.
    pub fn set_render(&mut self, enabled: bool) {
        self.nds.set_render(enabled);
    }

    /// Tell the core which screens anyone is looking at: bit 0 top, bit 1
    /// bottom. An engine whose screen is not shown does not compose it.
    pub fn set_displayed_screens(&mut self, mask: u8) {
        self.nds.set_displayed_screens(mask);
    }

    /// Hold or release white noise on the microphone.
    pub fn set_mic_static(&mut self, on: bool) {
        self.nds.set_mic_static(on);
    }

    /// Savestate path for one of the numbered slots, following melonDS's
    /// `<rom>.mlN` convention so the two front ends do not collide.
    pub fn state_path(&self, slot: u8) -> PathBuf {
        crate::file::settings::Settings::redirect(
            self.state_dir.as_ref(),
            &self.rom_path,
            &format!("ml{slot}"),
        )
    }

    /// Replace the cart's backup memory with `data` and restart, which is the
    /// only way the core takes a foreign save: it reads backup memory once, at
    /// construction.
    pub fn import_save(&mut self, data: &[u8]) -> Result<(), String> {
        std::fs::write(&self.saves.path, data)
            .map_err(|e| format!("cannot write {}: {e}", self.saves.path.display()))?;
        // Drop whatever the core was about to write, so the old save cannot
        // land on top of the imported one.
        *self.saves.pending.lock().unwrap() = None;
        // Rebooting must not cost this console its place on the airwaves: a
        // `Host` is fixed at construction, so a console rebooted without its
        // seat could never join one afterwards.
        let save_dir = self.saves.path.parent().map(Path::to_path_buf);
        let (instance_id, mp) = match self.seat.clone() {
            Some((id, mp)) => (id, Some(mp)),
            None => (0, None),
        };
        let reloaded = Self::boot_inner(
            &self.rom_path,
            save_dir.as_ref(),
            self.state_dir.as_ref(),
            instance_id,
            mp,
            None,
        )?;
        *self = reloaded;
        Ok(())
    }
}

impl Drop for Emu {
    /// A cart being unloaded is the last chance to persist its save.
    fn drop(&mut self) {
        self.flush_save();
    }
}
