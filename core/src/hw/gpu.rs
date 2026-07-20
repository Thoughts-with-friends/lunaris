//! NDS GPU – dual 2D engines plus one 3D engine.
//!
//! GBATEK references:
//! - Video overview: <https://problemkaputt.de/gbatek.htm#dsvideo>
//! - Display dimensions / timings: <https://problemkaputt.de/gbatek.htm#dsvideostuff>
//! - BG modes / DISPCNT: <https://problemkaputt.de/gbatek.htm#dsvideobgmodescontrol>
//! - 3D overview: <https://problemkaputt.de/gbatek.htm#ds3doverview>
//!
//! ## Display timing (GBATEK: "NDS Display Timings")
//! - Resolution per screen: **256 × 192** pixels.
//! - Total scanlines: **263** (lines 0-191 visible, 192-262 V-Blank).
//! - Dots per scanline: **355** (256 visible + 99 H-Blank).
//! - Master clock cycles per dot: **6** (33.513 MHz / 6 ≈ 5.585 MHz dot clock).
//! - H-Blank starts at dot **264** (256 + 8-dot delay) → scheduled at
//!   `HBLANK_DOT * CYCLES_PER_DOT`.
//!
//! ## Engines (GBATEK: "NDS 2D/3D Engines")
//! - **Engine A** (`engine_a`): supports BG0-BG3, OBJ, 3D output, display capture.
//! - **Engine B** (`engine_b`): BG0-BG3, OBJ only (no 3D, no capture).
//! - **Engine 3D** (`engine3d`): software-rendered 3D; output blended into Engine A's BG0.
//!
//! Engine A / B can be mapped to either the top or bottom LCD via POWCNT1.

pub mod debug;
mod engine2d;
mod engine3d;
mod registers;
mod vram;

use crate::hw::{
    HW, dma,
    interrupt_controller::{InterruptController, InterruptRequest},
    scheduler::{Event, Scheduler},
};

pub use engine2d::Engine2D;
pub use engine3d::Engine3D;
pub use registers::{DISPCAPCNT, DISPSTAT, DISPSTATFlags, POWCNT1};
pub use vram::VRAM;

use engine2d::DisplayMode;
use registers::CaptureSource;

#[derive(emu_utils::Savestate)]
pub struct GPU {
    // Registers and Values Shared between Engines
    pub dispstats: [DISPSTAT; 2],
    pub vcount: u16,
    #[savestate(skip)]
    rendered_frame: bool,

    pub engine_a: Engine2D<EngineA>,
    pub engine_b: Engine2D<EngineB>,
    pub engine3d: Engine3D,
    pub vram: VRAM,

    pub dispcapcnt: DISPCAPCNT,
    capturing: bool,
    pub powcnt1: POWCNT1,
}

impl GPU {
    pub const WIDTH: usize = 256; // Visible pixels per scanline
    pub const HEIGHT: usize = 192; // Visible scanlines per frame

    pub const PALETTE_SIZE: usize = 0x200; // 256 BGR555 entries × 2 bytes
    pub const OAM_SIZE: usize = 0x400; // 128 OBJ attributes × 8 bytes
    pub const OAM_MASK: usize = GPU::OAM_SIZE - 1;

    /// Master-clock cycles per display dot (33.513982 MHz / 6 = 5.585664 MHz
    /// dot clock).
    ///
    /// GBATEK "DS Video Stuff – DS Display Dimensions / Timings":
    /// <https://problemkaputt.de/gbatek.htm#dsvideostuff>
    const CYCLES_PER_DOT: usize = 6;
    /// H-Blank begins 8 dots after the last visible pixel (dot 264).
    ///
    /// GBATEK "H-Timing: 256 dots visible, 99 dots blanking":
    /// <https://problemkaputt.de/gbatek.htm#dsvideostuff>
    const HBLANK_DOT: usize = 256 + 8;
    /// Total dots per scanline: 256 visible + 99 H-Blank.
    ///
    /// GBATEK: <https://problemkaputt.de/gbatek.htm#dsvideostuff>
    const DOTS_PER_LINE: usize = 355;
    /// Total scanlines: 192 visible + 71 V-Blank.
    ///
    /// GBATEK "V-Timing: 192 lines visible, 71 lines blanking":
    /// <https://problemkaputt.de/gbatek.htm#dsvideostuff>
    const NUM_LINES: usize = 263;

    pub fn new(scheduler: &mut Scheduler) -> GPU {
        scheduler.schedule(Event::HBlank, HW::on_hblank, GPU::HBLANK_DOT * GPU::CYCLES_PER_DOT);
        GPU {
            // Registers and Values Shared between Engines
            dispstats: [DISPSTAT::new(), DISPSTAT::new()],
            vcount: 0,
            rendered_frame: false,

            engine_a: Engine2D::new(),
            engine_b: Engine2D::new(),
            engine3d: Engine3D::new(),
            vram: VRAM::new(),

            dispcapcnt: DISPCAPCNT::new(),
            capturing: false,
            powcnt1: POWCNT1::ENABLE_LCDS,
        }
    }

    /// Called at dot 0 of each new scanline.
    ///
    /// Clears the H-Blank flag in both DISPSTAT registers and advances VCOUNT.
    /// At scanline 262 (last V-Blank line) the affine BG reference points are
    /// re-latched, implementing the reference-point reload at V-Blank
    /// described in GBATEK "LCD I/O BG Rotation/Scaling":
    /// <https://problemkaputt.de/gbatek.htm#lcdiobgrotationscaling>
    ///
    /// DISPSTAT/VCOUNT semantics (same as GBA):
    /// <https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus>
    // Dot: 0 - TODO: Check for drift
    pub fn start_next_line(&mut self) {
        for dispstat in self.dispstats.iter_mut() {
            dispstat.remove(DISPSTATFlags::HBLANK)
        }

        if self.vcount == 262 {
            self.engine_a.latch_affine();
            self.engine_b.latch_affine();
        }
        self.vcount += 1;
        if self.vcount == GPU::NUM_LINES as u16 {
            self.vcount = 0;
        }
    }

    /// Called at dot 264 (start of H-Blank) to render the current scanline.
    ///
    /// Renders Engine A then Engine B when their respective POWCNT1 enable
    /// bits are set.  Display capture (DISPCAPCNT) is also processed here if
    /// active and the scanline is within the configured capture height.
    ///
    /// GBATEK "DS Video Capture and Main Memory Display Mode" (DISPCAPCNT,
    /// Engine A only):
    /// <https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode>
    // Dot: HBLANK_DOT - TODO: Check for drift
    pub fn render_line(&mut self) {
        // TODO: Use POWCNT to selectively render engines
        if self.powcnt1.contains(POWCNT1::ENABLE_ENGINE_A) {
            self.engine_a.render_line(&self.engine3d, &self.vram, self.vcount);
            if self.capturing && (self.vcount as usize) < self.dispcapcnt.capture_size.height() {
                self.capture();
            }
        }
        if self.powcnt1.contains(POWCNT1::ENABLE_ENGINE_B) {
            self.engine_b.render_line(&self.engine3d, &self.vram, self.vcount)
        }
    }

    /// Captures one scanline into VRAM according to DISPCAPCNT.
    ///
    /// Source A is Engine A output (or the raw 3D layer), source B is a VRAM
    /// block or the main-memory FIFO; sources can be blended with EVA/EVB.
    ///
    /// GBATEK "4000064h - NDS9 - DISPCAPCNT - 32bit":
    /// <https://problemkaputt.de/gbatek.htm#dsvideocaptureandmainmemorydisplaymode>
    pub fn capture(&mut self) {
        let start_addr = self.vcount as usize * GPU::WIDTH;
        let width = self.dispcapcnt.capture_size.width();
        fn get_engine_a_color(engine_a: &Engine2D<EngineA>, _: &Engine3D, index: usize) -> u16 {
            engine_a.pixels()[index]
        }
        fn get_engine3d_color(_: &Engine2D<EngineA>, engine_3d: &Engine3D, index: usize) -> u16 {
            engine_3d.pixel_color(index)
        }
        let src_a: fn(&Engine2D<EngineA>, &Engine3D, usize) -> u16 =
            if self.dispcapcnt.src_a_is_3d_only
                || self.engine_a.dispcnt.display_mode != DisplayMode::Mode0
            {
                get_engine3d_color
            } else {
                get_engine_a_color
            };
        let src_a_range = start_addr..start_addr + width;
        let mut src_b = [0; 2 * GPU::WIDTH];
        if self.dispcapcnt.src_b_fifo {
            todo!()
        } else {
            let offset = 2 * start_addr
                + if self.engine_a.dispcnt.display_mode == DisplayMode::Mode2 {
                    0
                } else {
                    self.dispcapcnt.vram_read_offset.offset()
                };
            let block = self.engine_a.dispcnt.vram_block as usize;
            // TODO: Figure out how to avoid this copy and keep borrow checker happy
            src_b[..2 * width].copy_from_slice(&self.vram.banks[block][offset..offset + 2 * width]);
        }

        let offset = 2 * start_addr + self.dispcapcnt.vram_write_offset.offset();
        let bank = &mut self.vram.banks[self.dispcapcnt.vram_write_block];
        // TODO: Replace write_mem and read_mem with slice conversions
        match self.dispcapcnt.capture_src {
            CaptureSource::A => {
                for (i, index) in src_a_range.enumerate() {
                    let pixel = src_a(&self.engine_a, &self.engine3d, index);
                    HW::write_mem(bank, offset as u32 + 2 * i as u32, pixel);
                }
            }
            CaptureSource::B => {
                bank[offset..offset + 2 * width].copy_from_slice(&src_b[..2 * width])
            }
            CaptureSource::AB => {
                for (i, a_index) in src_a_range.enumerate() {
                    let a_pixel = src_a(&self.engine_a, &self.engine3d, a_index);
                    let b_pixel = HW::read_mem::<u16>(&src_b, i as u32 * 2);
                    let a_alpha = a_pixel >> 15 & 0x1;
                    let b_alpha = b_pixel >> 15 & 0x1;
                    let mut intensity = 0;
                    // TODO: Move blending into a utility function
                    for i in (0..3).rev() {
                        let val_a = a_pixel >> (5 * i) & 0x1F;
                        let val_b = b_pixel >> (5 * i) & 0x1F;
                        let new_val = (val_a * a_alpha * self.dispcapcnt.eva as u16
                            + val_b * b_alpha * self.dispcapcnt.evb as u16)
                            / 16;
                        intensity = intensity << 5 | new_val;
                    }
                    let alpha = a_alpha != 0 && self.dispcapcnt.eva > 0
                        || b_alpha != 0 && self.dispcapcnt.evb > 0;
                    let final_pixel = (alpha as u16) << 15 | intensity;
                    HW::write_mem(bank, offset as u32 + 2 * i as u32, final_pixel);
                }
            }
        }
    }

    pub fn bus_stalled(&self) -> bool {
        self.engine3d.bus_stalled
    }

    pub fn rendered_frame(&mut self) -> bool {
        let rendered_frame = self.rendered_frame;
        self.rendered_frame = false;
        rendered_frame
    }

    pub fn get_screens(&self) -> [&Vec<u16>; 2] {
        if self.powcnt1.contains(POWCNT1::TOP_A) {
            [self.engine_a.pixels(), self.engine_b.pixels()]
        } else {
            [self.engine_b.pixels(), self.engine_a.pixels()]
        }
    }
}

impl HW {
    /// Scheduler handler for [`Event::StartNextLine`] (dot 0 of a scanline).
    ///
    /// Advances VCOUNT and manages the VBLANK flag: set on entering line 192,
    /// cleared on line 0.  Fires the V-Blank IRQ and the VCOUNT-match IRQ
    /// when the corresponding DISPSTAT enable bits are set.
    ///
    /// GBATEK "DISPSTAT / VCOUNT" (same layout as GBA):
    /// <https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus>
    /// V-Blank timing: <https://problemkaputt.de/gbatek.htm#dsvideostuff>
    pub fn start_next_line(&mut self, _event: Event) {
        self.scheduler.schedule(
            Event::HBlank,
            HW::on_hblank,
            GPU::HBLANK_DOT * GPU::CYCLES_PER_DOT,
        );
        self.gpu.start_next_line();
        if self.gpu.vcount == 0 {
            self.gpu.capturing = self.gpu.dispcapcnt.enable;
            for dispstat in self.gpu.dispstats.iter_mut() {
                dispstat.remove(DISPSTATFlags::VBLANK)
            }
        } else if self.gpu.vcount == GPU::HEIGHT as u16 {
            if self.gpu.capturing {
                self.gpu.dispcapcnt.enable = false
            }
            for dispstat in self.gpu.dispstats.iter_mut() {
                dispstat.insert(DISPSTATFlags::VBLANK)
            }
            self.gpu.rendered_frame = true;

            self.on_vblank(Event::VBlank);
            self.check_dispstats(&mut |dispstat, interrupts| {
                if dispstat.contains(DISPSTATFlags::VBLANK_IRQ_ENABLE) {
                    interrupts.request |= InterruptRequest::VBLANK;
                }
            });
        }

        let vcount = self.gpu.vcount;
        self.check_dispstats(&mut |dispstat, interrupts| {
            if dispstat.contains(DISPSTATFlags::VBLANK_IRQ_ENABLE)
                && vcount == dispstat.vcount_setting
            {
                interrupts.request |= InterruptRequest::VCOUNTER_MATCH;
            }
        });
    }

    /// Scheduler handler for [`Event::HBlank`] (dot 264 of a scanline).
    ///
    /// Sets the HBLANK flag, renders the visible scanline, starts
    /// H-Blank-triggered DMA (visible lines only), and raises the H-Blank
    /// IRQ if enabled.
    ///
    /// GBATEK H-Blank flag/IRQ:
    /// <https://problemkaputt.de/gbatek.htm#lcdiointerruptsandstatus>
    /// H-Blank DMA start timing:
    /// <https://problemkaputt.de/gbatek.htm#dsdmatransfers>
    pub fn on_hblank(&mut self, _event: Event) {
        self.scheduler.schedule(
            Event::StartNextLine,
            HW::start_next_line,
            (GPU::DOTS_PER_LINE - GPU::HBLANK_DOT) * GPU::CYCLES_PER_DOT,
        );
        for dispstat in self.gpu.dispstats.iter_mut() {
            dispstat.insert(DISPSTATFlags::HBLANK)
        }
        if self.gpu.vcount < GPU::HEIGHT as u16 {
            self.gpu.render_line();
            self.run_dmas_both(dma::Occasion::HBlank);
        }
        self.check_dispstats(&mut |dispstat, interrupts| {
            if dispstat.contains(DISPSTATFlags::HBLANK_IRQ_ENABLE) {
                interrupts.request |= InterruptRequest::HBLANK;
            }
        });
    }

    /// Runs once per frame at the start of V-Blank (line 192).
    ///
    /// Starts V-Blank-triggered DMA, then resolves the pending SwapBuffers
    /// and lets the 3D engine rasterize the frame (when enabled) and
    /// execute buffered geometry commands. Real hardware renders the 3D
    /// scene during V-Blank into line buffers for the next frame.
    ///
    /// The SwapBuffers resolution itself is unconditional: POWCNT1
    /// "Enable 3D Rendering" (bit 2) only gates the rasterizer, not the
    /// geometry engine's halt-until-VBlank behavior. Gating the resolution
    /// on that bit would leave the geometry engine (and GXFIFO) stalled
    /// forever whenever a game toggles rendering off mid-scene. See
    /// `docs/design/3d-rendering-bugfix-design.md` §3.1.
    ///
    /// GBATEK "DS 3D Overview" (rendering starts after VBlank):
    /// <https://problemkaputt.de/gbatek.htm#ds3doverview>
    /// V-Blank DMA: <https://problemkaputt.de/gbatek.htm#dsdmatransfers>
    pub fn on_vblank(&mut self, _event: Event) {
        self.run_dmas_both(dma::Occasion::VBlank);
        // TODO: Render using multiple threads
        let rendering_enabled = self.gpu.powcnt1.contains(POWCNT1::ENABLE_3D_RENDERING);
        self.gpu.engine3d.render(&self.gpu.vram, rendering_enabled);

        self.gpu.engine3d.exec_commands(&mut self.interrupts[1].request);
        self.check_geometry_command_fifo();
    }

    fn check_dispstats<F>(&mut self, check: &mut F)
    where
        F: FnMut(&mut DISPSTAT, &mut InterruptController),
    {
        for i in 0..2 {
            check(&mut self.gpu.dispstats[i], &mut self.interrupts[i])
        }
    }
}

pub trait EngineType {
    fn is_a() -> bool;
}
#[derive(emu_utils::Savestate)]
pub struct EngineA {}
#[derive(emu_utils::Savestate)]
pub struct EngineB {}

impl EngineType for EngineA {
    fn is_a() -> bool {
        true
    }
}
impl EngineType for EngineB {
    fn is_a() -> bool {
        false
    }
}
