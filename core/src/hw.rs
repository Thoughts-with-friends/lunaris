//! Hardware layer: wires together all NDS peripherals behind a single [`HW`] struct.
//!
//! Memory is accessed through two separate page-table fast-paths
//! (`arm7_page_table` / `arm9_page_table`) that map 4 KiB pages to raw
//! pointers so the hot read/write path avoids repeated address decoding.
//!
//! GBATEK references:
//! - Memory maps (ARM9/ARM7): <https://problemkaputt.de/gbatek.htm#dsmemorymaps>
//! - I/O maps: <https://problemkaputt.de/gbatek.htm#dsiomaps>
//! - Technical data (clock rates, RAM sizes): <https://problemkaputt.de/gbatek.htm#dstechnicaldata>

mod ar;
mod cartridge;
mod dma;
mod gpu;
mod interrupt_controller;
mod ipc;
mod keypad;
mod math;
pub mod mem;
mod rtc;
mod scheduler;
mod spi;
mod spu;
mod timers;

use std::convert::TryInto;
use std::fs::File;

use crate::{CheatMap, unlikely};
use cartridge::Cartridge;
pub use gpu::{EngineA, EngineB, GPU};
use interrupt_controller::{InterruptController, InterruptRequest};
use ipc::IPC;
pub use keypad::Key;
use keypad::Keypad;
use math::{Div, Sqrt};
pub use mem::{AccessType, MemoryValue};
use mem::{CP15, EXMEM, HALTCNT, POWCNT2, WRAMCNT};
use rtc::RTC;
use scheduler::{Event, EventHandler, Scheduler};
use spi::SPI;
use spu::SPU;
use timers::Timers;

/// Aggregated hardware state of the Nintendo DS.
///
/// # Memory layout
/// | Region      | Size    | Description                        |
/// |-------------|---------|------------------------------------|
/// | `main_mem`  | 4 MiB   | ARM9 main RAM (mirrored)           |
/// | `iwram`     | 64 KiB  | ARM7 internal WRAM                 |
/// | `shared_wram`| 32 KiB | Configurable ARM7/ARM9 shared WRAM |
/// | `itcm`      | 32 KiB  | ARM9 Instruction TCM               |
/// | `dtcm`      | 16 KiB  | ARM9 Data TCM                      |
///
/// # Page tables
/// `arm7_page_table` and `arm9_page_table` are raw-pointer caches over the
/// above buffers.  They are rebuilt on init and after every savestate load.
#[derive(emu_utils::Savestate)]
#[load(post = "self.post_load_hw(save)?", in_place_only)]
pub struct HW {
    #[savestate(skip)]
    pub enable_cheats: bool,
    // Cheats <addr, value>
    #[savestate(skip)] // external cheat file
    pub cheat_map: CheatMap,

    // Memory
    pub cp15: CP15,
    /// Not serialized: BIOS images are immutable and re-supplied by the host
    /// at construction time, so shipping ~20 KB of BIOS in every savestate
    /// is pure waste. See `docs/design/savestate-and-video-design.md` §1.3.
    #[savestate(skip)]
    bios7: Vec<u8>,
    #[savestate(skip)]
    bios9: Vec<u8>,
    cartridge: Cartridge,
    #[load(with_in_place = "*itcm = save.load()?")]
    itcm: Vec<u8>,
    #[load(with_in_place = "*dtcm = save.load()?")]
    dtcm: Vec<u8>,
    // #[savestate(skip)] // Skip Dust too
    #[load(with_in_place = "*main_mem = save.load()?")]
    pub main_mem: Vec<u8>,
    #[load(with_in_place = "*iwram = save.load()?")]
    iwram: Vec<u8>,
    #[load(with_in_place = "*shared_wram = save.load()?")]
    shared_wram: Vec<u8>,
    /// Raw-pointer page table for ARM7 memory (4 KiB pages). Not serialized.
    #[savestate(skip)]
    arm7_page_table: Vec<*mut u8>,
    /// Raw-pointer page table for ARM9 memory (4 KiB pages). Not serialized.
    #[savestate(skip)]
    arm9_page_table: Vec<*mut u8>,
    // Devices
    pub gpu: GPU,
    spu: SPU,
    keypad: Keypad,
    /// `[0]` = ARM7 interrupt controller, `[1]` = ARM9 interrupt controller.
    interrupts: [InterruptController; 2],
    /// `[0]` = ARM7 DMA controller, `[1]` = ARM9 DMA controller.
    dmas: [dma::Controller; 2],
    dma_fill: [u32; 4],
    /// `[0]` = ARM7 timers, `[1]` = ARM9 timers.
    timers: [Timers; 2],
    ipc: IPC,
    rtc: RTC,
    spi: SPI,
    // Registers
    wramcnt: WRAMCNT,
    powcnt2: POWCNT2,
    pub haltcnt: HALTCNT,
    postflg7: u8,
    postflg9: u8,
    exmem: EXMEM,
    // Math
    div: Div,
    sqrt: Sqrt,
    // Misc
    scheduler: Scheduler,
}

impl HW {
    fn post_load_hw<S: emu_utils::ReadSavestate>(&mut self, _save: &mut S) -> Result<(), S::Error> {
        self.scheduler.restore_events(HW::handler_for_event);
        self.init_arm7_page_tables();
        self.init_arm9_page_tables();
        // Clear 3D bus stall so CPUs always run after state load.
        // If the GXFIFO was full at save time, exec_commands at the next VBlank
        // will drain it; clearing the flag here prevents permanent CPU starvation.
        self.gpu.engine3d.bus_stalled = false;
        // Re-evaluate the GXFIFO IRQ condition. `check_interrupts` is normally
        // driven by register writes and command execution, so if the ARM9 was
        // `IntrWait`-ing on GEOMETRY_COMMAND_FIFO at save time, the edge that
        // would wake it up is otherwise lost across a save/load cycle.
        // See `docs/design/savestate-and-video-design.md` §2.3 (C3).
        self.gpu.engine3d.check_interrupts(&mut self.interrupts[1].request);
        Ok(())
    }

    /// Test-only: shifts the scheduler's and every timer's absolute cycle
    /// counters by `offset`, simulating a long play session for u32-overflow
    /// regression tests. See `docs/design/savestate-and-video-design.md` §3.4.
    #[cfg(test)]
    pub(crate) fn offset_cycles_for_test(&mut self, offset: usize) {
        self.scheduler.offset_cycle_for_test(offset);
        self.timers[0].offset_cycles_for_test(offset);
        self.timers[1].offset_cycles_for_test(offset);
    }

    fn handler_for_event(event: &Event) -> EventHandler {
        match event {
            Event::DMA(_, _) => HW::on_dma,
            Event::StartNextLine => HW::start_next_line,
            Event::HBlank => HW::on_hblank,
            Event::VBlank => HW::on_vblank,
            Event::CheckGeometryCommandFIFO => HW::check_geometry_command_fifo_handler,
            Event::TimerOverflow(_, _) => HW::on_timer_overflow,
            Event::ROMWordTransfered(_) => HW::on_rom_word_transfered,
            Event::ROMBlockEnded(_) => HW::on_rom_block_ended,
            Event::GenerateAudioSample => HW::generate_audio_sample,
            Event::StepAudioChannel(_) => HW::step_audio_channel,
            Event::ResetAudioChannel(_) => HW::reset_audio_channel,
        }
    }

    const ITCM_SIZE: usize = 0x8000; // 32 KiB
    const DTCM_SIZE: usize = 0x4000; // 16 KiB
    const MAIN_MEM_SIZE: usize = 0x40_0000; // 4 MiB
    const IWRAM_SIZE: usize = 0x1_0000; // 64 KiB
    const SHARED_WRAM_SIZE: usize = 0x8000; // 32 KiB

    pub fn new(
        bios7: Vec<u8>,
        bios9: Vec<u8>,
        firmware_file: File,
        rom: Vec<u8>,
        save_file: File,
        direct_boot: bool,
    ) -> Self {
        let mut scheduler = Scheduler::new();
        let cartridge = Cartridge::new(rom, save_file, &bios7);
        let mut hw = HW {
            enable_cheats: false,
            cheat_map: CheatMap::new(),
            // Memory
            cp15: CP15::new(),
            bios7,
            bios9,
            cartridge,
            itcm: vec![0; HW::ITCM_SIZE],
            dtcm: vec![0; HW::DTCM_SIZE],
            main_mem: vec![0; HW::MAIN_MEM_SIZE],
            iwram: vec![0; HW::IWRAM_SIZE],
            shared_wram: vec![0; HW::SHARED_WRAM_SIZE],
            arm7_page_table: vec![std::ptr::null_mut(); HW::ARM7_PAGE_TABLE_SIZE],
            arm9_page_table: vec![std::ptr::null_mut(); HW::ARM9_PAGE_TABLE_SIZE],
            // Devices
            gpu: GPU::new(&mut scheduler),
            spu: SPU::new(&mut scheduler),
            keypad: Keypad::new(),
            interrupts: [InterruptController::new(), InterruptController::new()],
            dmas: [dma::Controller::new(false), dma::Controller::new(true)],
            dma_fill: [0; 4],
            timers: [Timers::new(false), Timers::new(true)],
            ipc: IPC::new(),
            rtc: RTC::new(),
            spi: SPI::new(firmware_file),
            // Registesr
            wramcnt: WRAMCNT::new(3),
            powcnt2: POWCNT2::new(),
            haltcnt: HALTCNT::new(),
            postflg7: if direct_boot { 0x1 } else { 0x0 },
            postflg9: if direct_boot { 0x1 } else { 0x0 },
            exmem: EXMEM::new(),
            // Math
            div: Div::new(),
            sqrt: Sqrt::new(),
            // Misc
            scheduler,
        };
        hw.init_arm7_page_tables();
        hw.init_arm9_page_tables();
        if direct_boot {
            hw.init_mem()
        } else {
            hw.cartridge.encrypt_secure_area();
            hw
        }
    }

    /// Advances the scheduler to `target` cycles and fires any pending events.
    pub fn clock_until(&mut self, target: usize) {
        self.handle_events(target);
    }

    /// Returns `true` if the ARM7 has pending, unmasked interrupts.
    ///
    /// Also propagates keypad interrupt requests into the controller.
    pub fn arm7_interrupts_requested(&mut self) -> bool {
        if unlikely(self.keypad.interrupt_requested()) {
            self.interrupts[0].request |= InterruptRequest::KEYPAD
        }
        self.interrupts[0].interrupts_requested(self.haltcnt.halted())
    }

    /// Returns `true` if the ARM9 has pending, unmasked interrupts.
    pub fn arm9_interrupts_requested(&mut self) -> bool {
        if unlikely(self.keypad.interrupt_requested()) {
            self.interrupts[1].request |= InterruptRequest::KEYPAD
        }
        self.interrupts[1].interrupts_requested(false)
    }

    pub fn rendered_frame(&mut self) -> bool {
        self.gpu.rendered_frame()
    }

    /// Returns a compact fingerprint of the currently loaded ROM.
    ///
    /// Used to verify that a savestate file was captured against the ROM
    /// currently loaded before applying it, since [`Cartridge::rom`] itself
    /// is no longer part of the savestate (see
    /// `docs/design/savestate-and-video-design.md` §1.3).
    pub fn rom_fingerprint(&self) -> RomFingerprint {
        let header = self.cartridge.header();
        RomFingerprint {
            game_code: header.game_code,
            header_checksum: header.header_checksum,
            secure_area_checksum: header.secure_area_checksum,
            rom_len: self.cartridge.rom().len() as u64,
        }
    }

    pub fn press_key(&mut self, key: Key) {
        self.keypad.press_key(key);
    }

    pub fn release_key(&mut self, key: Key) {
        self.keypad.release_key(key);
    }

    pub fn press_screen(&mut self, x: usize, y: usize) {
        self.keypad.press_screen();
        self.spi.press_screen(x, y)
    }

    pub fn release_screen(&mut self) {
        self.keypad.release_screen();
        self.spi.release_screen();
    }

    pub fn set_audio_volume(&mut self, volume_percent: f32) {
        self.spu.set_audio_volume(volume_percent);
    }

    pub fn render_palettes(
        &self,
        extended: bool,
        slot: usize,
        palette: usize,
        engine: Engine,
        graphics_type: GraphicsType,
    ) -> (Vec<u16>, usize, usize) {
        if extended {
            match (engine, graphics_type) {
                (Engine::A, GraphicsType::BG) => GPU::render_palettes(
                    |i| self.gpu.vram.get_bg_ext_pal::<EngineA>(slot, palette * 256 + i),
                    16,
                ),
                (Engine::A, GraphicsType::OBJ) => GPU::render_palettes(
                    |i| self.gpu.vram.get_obj_ext_pal::<EngineA>(palette * 256 + i),
                    16,
                ),
                (Engine::B, GraphicsType::BG) => GPU::render_palettes(
                    |i| self.gpu.vram.get_bg_ext_pal::<EngineB>(slot, palette * 256 + i),
                    16,
                ),
                (Engine::B, GraphicsType::OBJ) => GPU::render_palettes(
                    |i| self.gpu.vram.get_obj_ext_pal::<EngineB>(palette * 256 + i),
                    16,
                ),
            }
        } else {
            match (engine, graphics_type) {
                (Engine::A, GraphicsType::BG) => {
                    GPU::render_palettes(|i| self.gpu.engine_a.bg_palettes()[i], 16)
                }
                (Engine::A, GraphicsType::OBJ) => {
                    GPU::render_palettes(|i| self.gpu.engine_a.obj_palettes()[i], 16)
                }
                (Engine::B, GraphicsType::BG) => {
                    GPU::render_palettes(|i| self.gpu.engine_b.bg_palettes()[i], 16)
                }
                (Engine::B, GraphicsType::OBJ) => {
                    GPU::render_palettes(|i| self.gpu.engine_b.obj_palettes()[i], 16)
                }
            }
        }
    }

    pub fn render_map(&self, engine: Engine, bg_i: usize) -> (Vec<u16>, usize, usize) {
        match engine {
            Engine::A => self.gpu.engine_a.render_map(&self.gpu.vram, bg_i),
            Engine::B => self.gpu.engine_b.render_map(&self.gpu.vram, bg_i),
        }
    }

    pub fn render_tiles(
        &self,
        engine: Engine,
        graphics_type: GraphicsType,
        extended: bool,
        bitmap: bool,
        bpp8: bool,
        slot: usize,
        palette: usize,
        offset: usize,
    ) -> (Vec<u16>, usize, usize) {
        let is_bg = graphics_type == GraphicsType::BG;
        match engine {
            Engine::A => self.gpu.engine_a.render_tiles(
                &self.gpu.vram,
                is_bg,
                extended,
                bitmap,
                bpp8,
                slot,
                palette,
                offset,
            ),
            Engine::B => self.gpu.engine_b.render_tiles(
                &self.gpu.vram,
                is_bg,
                extended,
                bitmap,
                bpp8,
                slot,
                palette,
                offset,
            ),
        }
    }

    pub fn render_bank(&self, ignore_alpha: bool, bank: usize) -> (Vec<u16>, usize, usize) {
        self.gpu.vram.render_bank(ignore_alpha, bank)
    }

    /// Populates main memory with the ROM header and boot-info words needed for
    /// direct-boot (skipping the firmware splash screen).
    ///
    /// Mirrors the setup performed by the NDS BIOS during a normal cold boot:
    /// the cartridge header is copied to 27FFE00h, and the chip ID / secure
    /// area CRC words are placed at 27FF800h/27FFC00h.
    ///
    /// GBATEK "DS Cartridge Header – header is loaded to 27FFE00h" and RAM
    /// boot values: <https://problemkaputt.de/gbatek.htm#dscartridgeheader>
    /// (see also "DS Memory Maps – Main Memory boot/debug area":
    /// <https://problemkaputt.de/gbatek.htm#dsmemorymaps>)
    pub fn init_mem(mut self) -> Self {
        let addr = 0x027F_FE00 & (HW::MAIN_MEM_SIZE - 1);
        self.main_mem[addr..addr + 0x170].copy_from_slice(&self.cartridge.rom()[..0x170]);

        for addr in [0x027FF800, 0x027FFC00].iter() {
            self.arm9_write(*addr, self.cartridge.chip_id());
            self.arm9_write(addr + 0x4, self.cartridge.chip_id());
            self.arm9_write(
                addr + 0x8,
                u16::from_le_bytes(self.cartridge.rom()[0x15E..=0x15F].try_into().unwrap()),
            );
            self.arm9_write(
                addr + 0xA,
                u16::from_le_bytes(self.cartridge.rom()[0x6C..=0x6D].try_into().unwrap()),
            );
        }

        self.arm9_write(0x027FF850, 0x5835u16);
        self.arm9_write(0x027FFC10, 0x5835u16);
        self.arm9_write(0x027FFC30, 0xFFFFu16);
        self.arm9_write(0x027FFC40, 0x0001u16);
        self
    }

    /// Fills `page_table` entries for the address range `[addr_start, addr_end)`
    /// so that each 4 KiB page slot points directly into `mem` (with mirroring).
    fn map_page_table(
        page_table: &mut [*mut u8],
        page_shift: usize,
        page_size: usize,
        addr_start: usize,
        addr_end: usize,
        mem: &mut [u8],
    ) {
        let mem_mask = mem.len() - 1;
        for (page_table_i, addr) in
            (addr_start >> page_shift..).zip((addr_start..addr_end).step_by(page_size))
        {
            let mem_addr = addr & mem_mask;
            page_table[page_table_i] = mem[mem_addr..mem_addr + page_size].as_mut_ptr();
        }
    }
}

/// Compact fingerprint of a loaded ROM: game code plus the two checksum
/// fields already present in the cartridge header, plus ROM length.
///
/// Used to verify a savestate file matches the ROM currently loaded, without
/// re-hashing the (potentially 100+ MB) ROM on every save/load. See
/// `docs/design/savestate-and-video-design.md` §1.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomFingerprint {
    pub game_code: u32,
    pub header_checksum: u16,
    pub secure_area_checksum: u16,
    pub rom_len: u64,
}

impl RomFingerprint {
    /// Fixed on-disk encoding length in bytes (see [`RomFingerprint::to_bytes`]).
    pub const ENCODED_LEN: usize = 16;

    pub fn to_bytes(self) -> [u8; Self::ENCODED_LEN] {
        let mut out = [0u8; Self::ENCODED_LEN];
        out[0..4].copy_from_slice(&self.game_code.to_le_bytes());
        out[4..6].copy_from_slice(&self.header_checksum.to_le_bytes());
        out[6..8].copy_from_slice(&self.secure_area_checksum.to_le_bytes());
        out[8..16].copy_from_slice(&self.rom_len.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8; Self::ENCODED_LEN]) -> Self {
        RomFingerprint {
            game_code: u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            header_checksum: u16::from_le_bytes(bytes[4..6].try_into().unwrap()),
            secure_area_checksum: u16::from_le_bytes(bytes[6..8].try_into().unwrap()),
            rom_len: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        }
    }
}

/// Selects one of the two 2-D graphics engines.
#[derive(Clone, Copy, PartialEq)]
pub enum Engine {
    /// Engine A – can display 2-D and 3-D content; connected to either screen.
    A = 0,
    /// Engine B – 2-D only; connected to the other screen.
    B = 1,
}

impl Engine {
    pub fn label(&self) -> &str {
        match self {
            Engine::A => "A",
            Engine::B => "B",
        }
    }
}

/// Distinguishes background layers from sprite (OBJ) layers in GPU debug APIs.
#[derive(Clone, Copy, PartialEq)]
pub enum GraphicsType {
    BG,  // Background
    OBJ, // Object: character sprites
}

impl GraphicsType {
    pub fn label(&self) -> &str {
        match self {
            GraphicsType::BG => "BG",
            GraphicsType::OBJ => "OBJ",
        }
    }
}
