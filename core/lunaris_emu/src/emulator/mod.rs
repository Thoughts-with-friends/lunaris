// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
//! Core emulator system that manages CPU, memory, and all peripheral devices
//! Handles the dual-CPU architecture of the Nintendo DS and system timing
pub mod emu_config;
mod read_arm7;
mod read_arm9;
mod write_arm7;
mod write_arm9;

use snafu::ResultExt as _;
use std::collections::VecDeque;

use lunaris_ds_audio::SPU;
use lunaris_ds_cpu::arm_cpu::ArmCpu;
use lunaris_ds_gpu::gpu_root::{Gpu, gpu_reg::SchedulerEvent};
use lunaris_ds_mem_const::*;

use crate::bios::Bios;
use crate::cartridge::NDSCart;
use crate::cp15::Cp15;
use crate::dma::NDSDma;
use crate::error::{EmuError, FailedReadFileSnafu};
use crate::interrupts::{Interrupt, InterruptRegs};
use crate::ipc::{IpcFifo, IpcSync};
use crate::rtc::RealTimeClock;
use crate::spi::SPIBus;
use crate::timers::NDSTiming;
use crate::wifi::WiFi;
use emu_config::{BiosMem, Config, ExtKeyInReg, KeyInputReg, PowCnt2Reg};

/// Core Nintendo DS emulator system
/// Manages dual ARM CPUs, memory, and all peripheral devices
#[derive(Debug, Default)]
pub struct Emulator {
    /// Config
    pub config: Config,

    pub cycle_count: u64,
    pub arm7: ArmCpu,
    pub arm9: ArmCpu,
    pub bios: Bios,
    pub arm9_cp15: Cp15,
    pub cart: NDSCart,
    pub dma: NDSDma,
    pub gpu: Gpu,
    pub rtc: RealTimeClock,
    pub spi: SPIBus,
    pub spu: SPU,
    pub timers: NDSTiming,
    pub wifi: WiFi,

    /// Main system RAM (4MB)
    pub main_ram: Vec<u8>,
    /// Shared CPU WRAM (32KB)
    pub shared_wram: Vec<u8>,
    /// ARM7-only WRAM (64KB)
    pub arm7_wram: Vec<u8>,

    pub arm9_bios: BiosMem<BIOS9_SIZE>,
    pub arm7_bios: BiosMem<BIOS7_SIZE>,

    /// Scheduling
    pub system_timestamp: u64,
    pub next_event_time: u64,
    pub gpu_event: SchedulerEvent,
    pub dma_event: SchedulerEvent,

    /// IPC and FIFO
    pub ipc_sync_nds9: IpcSync,
    pub ipc_sync_nds7: IpcSync,
    pub fifo7: IpcFifo,
    pub fifo9: IpcFifo,

    pub fifo7_queue: VecDeque<u32>, // std::queue<uint32_t>
    pub fifo9_queue: VecDeque<u32>, // std::queue<uint32_t>
    pub aux_spi_cnt: u16,

    pub int7_reg: InterruptRegs,
    pub int9_reg: InterruptRegs,

    /// Input state - standard buttons
    pub key_input: KeyInputReg,
    /// Input state - extended buttons
    pub ext_key_in: ExtKeyInReg,

    /// Power control register
    pub pow_cnt2: PowCnt2Reg,

    /// DMA fill values for each of 4 DMA units
    pub dma_fill: [u32; 4],
    /// Serial I/O control register
    /// - 0x04000128
    pub sio_cnt: u16,
    /// External memory control
    /// - 0x04000134
    pub r_cnt: u16,
    pub ex_mem_cnt: u16,
    /// - 0x04000247
    pub wram_cnt: u8,

    /// Division engine registers
    /// - 0x04000280
    pub divcnt: u16,
    pub div_numer: u64,
    pub div_denom: u64,
    pub div_result: u64,
    pub div_remresult: u64,

    /// Square root engine registers
    pub sqrtcnt: u16,
    pub sqrt_result: u32,
    pub sqrt_param: u64,

    /// Power-on flags for debugging/BIOS
    /// - 0x04000300
    pub postflg7: u8,
    pub postflg9: u8,

    /// BIOS protection register
    /// - 0x04000308
    pub bios_prot: u32,

    /// Debugging purposes
    pub hstep_even: bool,

    /// Current instruction cycle counter
    pub cycles: i32,

    /// Total system timestamp in cycles
    pub total_timestamp: u64,
    /// Last ARM9 execution timestamp
    pub last_arm9_timestamp: u64,
    /// Last ARM7 execution timestamp
    pub last_arm7_timestamp: u64,
}

impl Emulator {
    /// Create a new emulator instance with default values
    pub fn new() -> Self {
        Emulator {
            ..Default::default()
        }
    }

    /* ===== Internal helpers (private) ===== */

    /// Check FIFO interrupt for ARM7.
    ///
    /// NOTE: It is not used in C++ and has no definition.
    fn check_fifo7_interrupt(&mut self) {
        unimplemented!("It is not used in C++ and has no definition.");
    }

    /// Check FIFO interrupt for ARM9.
    ///
    /// NOTE: It is not used in C++ and has no definition.
    fn check_fifo9_interrupt(&mut self) {
        unimplemented!("It is not used in C++ and has no definition.");
    }

    /// Start hardware division unit.
    /// Start division operation
    pub fn start_division(&mut self) {
        if self.div_denom != 0 {
            self.div_result = self.div_numer / self.div_denom;
            self.div_remresult = self.div_numer % self.div_denom;
        }
    }

    /// Start hardware square root unit.
    pub fn start_sqrt(&mut self) {
        self.sqrt_result = (self.sqrt_param as f64).sqrt() as u32;
    }

    /* ===== load (public) ===== */

    /// Initialize emulator subsystems.
    pub fn init(&mut self) -> i32 {
        // self.arm9.set_cp15(&self.arm9_cp15);
        // self.fifo7.receive_queue = &self.fifo7_queue;
        // self.fifo7.send_queue = &self.fifo9_queue;

        // self.fifo9.receive_queue = &self.fifo9_queue;
        // self.fifo9.send_queue = &self.fifo7_queue;
        0
    }

    /// Load firmware from internal source.
    pub fn load_firmware(&mut self) -> Result<(), EmuError> {
        let bin =
            std::fs::read(&self.config.arm9_bios_path).with_context(|_| FailedReadFileSnafu {
                path: &self.config.arm9_bios_path,
            })?;
        self.arm9_bios = BiosMem::User(bin);

        let bin =
            std::fs::read(&self.config.arm7_bios_path).with_context(|_| FailedReadFileSnafu {
                path: &self.config.arm9_bios_path,
            })?;
        self.arm7_bios = BiosMem::User(bin);

        self.spi.init(&self.config.firmware_path)
    }

    /// Load GBA BIOS.
    pub fn load_bios_gba(&mut self, bios: &[u8]) {
        unimplemented!("It is used in v2.");
    }

    /// Load ARM7 BIOS.
    pub fn load_bios7(&mut self, bios: &[u8]) {
        unimplemented!("It is not used in C++ and has no definition.");
    }

    /// Load ARM9 BIOS.
    pub fn load_bios9(&mut self, bios: &[u8]) {
        unimplemented!("It is not used in C++ and has no definition.");
    }

    /// Load firmware image.
    pub fn load_firmware_image(&mut self, firmware: &[u8]) {
        unimplemented!();
    }

    /// Load Slot-2 cartridge data.
    pub fn load_slot2(&mut self, data: &[u8]) {
        unimplemented!();
    }

    /// Load save database by name.
    pub fn load_save_database(&mut self, name: &str) {
        unimplemented!();
    }

    /// Load a ROM file.
    pub fn load_rom(&mut self, rom_name: &str) -> i32 {
        unimplemented!();
    }

    /* ===== mark (public) ===== */

    /// Mark an address as ARM instruction.
    pub fn mark_as_arm(&mut self, address: u32) {
        unimplemented!();
    }

    /// Mark an address as THUMB instruction.
    pub fn mark_as_thumb(&mut self, address: u32) {
        unimplemented!();
    }

    /* ===== power and run (public) ===== */

    /// Power on the system.
    pub fn power_on(&mut self) {
        unimplemented!();
    }

    /// Perform a direct boot sequence.
    pub fn direct_boot(&mut self) {
        // Write zero to boot flag
        self.arm7_write_word(0x027FF864, 0);

        // Write shifted value from firmware[0x20]
        let value_0x20 = u16::from_le_bytes([
            self.spi.firmware.raw_firmware[0x20],
            self.spi.firmware.raw_firmware[0x21],
        ]);
        self.arm7_write_word(0x027FF868, (value_0x20 as u32) << 3);

        // Write halfword from firmware[0x26]
        let value_0x26 = u16::from_le_bytes([
            self.spi.firmware.raw_firmware[0x26],
            self.spi.firmware.raw_firmware[0x27],
        ]);
        self.arm7_write_halfword(0x027FF874, value_0x26);

        // Write halfword from firmware[0x04]
        let value_0x04 = u16::from_le_bytes([
            self.spi.firmware.raw_firmware[0x04],
            self.spi.firmware.raw_firmware[0x05],
        ]);
        self.arm7_write_halfword(0x027FF876, value_0x04);

        // Copy USER data block (0x70 bytes, word-aligned)
        for i in (0..0x70).step_by(4) {
            let offset = (self.spi.firmware.user_data as usize) + i;

            let word = u32::from_le_bytes([
                self.spi.firmware.raw_firmware[offset],
                self.spi.firmware.raw_firmware[offset + 1],
                self.spi.firmware.raw_firmware[offset + 2],
                self.spi.firmware.raw_firmware[offset + 3],
            ]);

            self.arm7_write_word(0x027FFC80 + i as u32, word);
        }
    }

    /// Run emulator in debug mode.
    pub fn debug(&mut self) {
        unimplemented!();
    }

    /// Run the emulator main loop.
    pub fn run(&mut self) {
        unimplemented!();
    }

    /// Run emulator in GBA mode.
    pub fn run_gba(&mut self) {
        unimplemented!();
    }

    /// Check whether a CPU is requesting an interrupt.
    pub fn requesting_interrupt(&self, cpu_id: u32) -> bool {
        match cpu_id {
            0 => {
                (self.int9_reg.irq_enable & self.int9_reg.irq_flags) != 0 && self.int9_reg.ime != 0
            }
            _ => {
                (self.int7_reg.irq_enable & self.int7_reg.irq_flags) != 0 && self.int7_reg.ime != 0
            }
        }
    }

    /* ===== GBA (public) ===== */

    /// Check if emulator is currently in GBA mode.
    pub fn is_gba(&self) -> bool {
        unimplemented!();
    }

    /// Start GBA mode.
    pub fn start_gba_mode(&mut self, throw_exception: bool) {
        unimplemented!();
    }

    /// Get current system timestamp.
    pub fn get_timestamp(&self) -> u64 {
        self.total_timestamp
    }

    /* ===== get frame(public) ===== */

    /// Copy upper screen framebuffer.
    pub fn get_upper_frame(&self) -> Vec<u32> {
        // Return upper screen pixel data
        vec![0u32; 256 * 192]
    }

    /// Copy lower screen framebuffer.
    pub fn get_lower_frame(&self) -> Vec<u32> {
        // Return lower screen pixel data
        vec![0u32; 256 * 192]
    }

    /// Set upper screen framebuffer.
    pub fn set_upper_screen(&mut self, buffer: &[u32]) {
        unimplemented!();
    }

    /// Set lower screen framebuffer.
    pub fn set_lower_screen(&mut self, buffer: &[u32]) {
        unimplemented!();
    }

    /* frame, display and dma ===== (public) ===== */

    /// Check if a frame has completed.
    pub fn frame_complete(&self) -> bool {
        unimplemented!();
    }

    /// Check if screens are swapped.
    pub fn display_swapped(&self) -> bool {
        unimplemented!();
    }

    /// Check if DMA is active for a CPU.
    pub fn dma_active(&self, cpu_id: i32) -> bool {
        unimplemented!();
    }

    /* DMA ===== (public) ===== */

    /// Request HBLANK DMA.
    pub fn hblank_dma_request(&mut self) {
        unimplemented!();
    }

    /// Request game cartridge DMA.
    pub fn gamecart_dma_request(&mut self) {
        unimplemented!();
    }

    /// Request GX FIFO DMA.
    pub fn gxfifo_dma_request(&mut self) {
        unimplemented!();
    }

    /// Check GX FIFO DMA status.
    pub fn check_gxfifo_dma(&mut self) {
        self.gpu.check_gxfifo_dma();
    }

    /* ===== add and sys stamp (public) ===== */

    /// Add a GPU event.
    pub fn add_gpu_event(&mut self, event_id: i32, relative_time: u64) {
        self.gpu_event.id = event_id;
        self.gpu_event.activation_time = self.system_timestamp + relative_time;

        if self.gpu_event.activation_time < self.next_event_time {
            self.next_event_time = self.gpu_event.activation_time;
        }
    }

    /// Add a DMA event.
    pub fn add_dma_event(&mut self, event_id: i32, relative_time: u64) {
        self.dma_event.id = event_id;
        self.dma_event.processing = true;
        self.dma_event.activation_time = self.system_timestamp + relative_time;

        if self.dma_event.activation_time < self.next_event_time {
            self.next_event_time = self.dma_event.activation_time;
        }
    }

    /// Recalculate system timestamp.
    pub fn calculate_system_timestamp(&mut self) {
        let cycles = self.next_event_time - self.system_timestamp;
        match cycles {
            0 | 20.. => self.system_timestamp += 20,
            _ => self.system_timestamp += cycles,
        }
    }

    /* ===== touchscreen (public) ===== */

    /// Handle touchscreen press.
    pub fn touchscreen_press(&mut self, x: i32, y: i32) {
        self.ext_key_in.pen_down = y != 0xfff;
        self.spi.touchscreen_press(x, y);
    }

    /// Call high-level BIOS function.
    pub fn hle_bios(&mut self, cpu_id: i32) -> i32 {
        match cpu_id {
            0 => self.bios.swi9(&mut self.arm9),
            _ => self.bios.swi7(&mut self.arm7),
        }
    }

    /* ===== read and write arm (public) =====
      - moved read_arm7/read_arm9, write_arm7/write_arm9
    */

    /* ===== cartrige (public) ===== */

    pub fn cart_copy_keybuffer(&self, buffer: &mut [u8]) {
        if let Some(bios) = &self.arm7_bios.get(0x30..0x30 + 0x1048) {
            buffer[..0x1048].copy_from_slice(bios);
        } else {
            #[cfg(feature = "tracing")]
            tracing::warn!("arm7_bios need: 0x30..0x30 + 0x1048");
        }
    }

    pub fn cart_write_header(&mut self, address: u32, halfword: u16) {
        let addr = ((0x027FFE00 + (address & 0x1FF)) & MAIN_RAM_MASK) as usize;

        if addr + 1 < self.main_ram.len() {
            let bytes = halfword.to_le_bytes();
            self.main_ram[addr] = bytes[0];
            self.main_ram[addr + 1] = bytes[1];
        }
    }

    /* ===== interrupt (public) ===== */

    /// Request an interrupt for ARM7.
    pub fn request_interrupt7(&mut self, id: Interrupt) {
        self.int7_reg.irq_flags |= 1 << (id as u32);
    }

    /// Request an interrupt for ARM9.
    pub fn request_interrupt9(&mut self, id: Interrupt) {
        self.int9_reg.irq_flags |= 1 << (id as u32);
    }

    /// Request a GBA interrupt.
    pub fn request_interrupt_gba(&mut self, id: i32) {
        self.int7_reg.irq_flags |= 1 << (id as u32);
    }

    /// Check if ARM7 has cartridge access rights.
    pub fn arm7_has_cart_rights(&self) -> bool {
        todo!()
    }

    /* ===== Button input handling (public) ===== */

    /// Handle up button press
    pub fn button_up_pressed(&mut self) {
        self.key_input.up = true;
    }

    /// Handle down button press
    pub fn button_down_pressed(&mut self) {
        self.key_input.down = true;
    }

    /// Handle left button press
    pub fn button_left_pressed(&mut self) {
        self.key_input.left = true;
    }

    /// Handle start button press
    pub fn button_start_pressed(&mut self) {
        self.key_input.start = true;
    }

    /// Handle select button press
    pub fn button_select_pressed(&mut self) {
        self.key_input.select = true;
    }

    /// Handle A button press
    pub fn button_a_pressed(&mut self) {
        self.key_input.button_a = true;
    }

    /// Handle B button press
    pub fn button_b_pressed(&mut self) {
        self.key_input.button_b = true;
    }

    /// Handle X button press
    pub fn button_x_pressed(&mut self) {
        self.ext_key_in.button_x = true;
    }

    /// Handle Y button press
    pub fn button_y_pressed(&mut self) {
        self.ext_key_in.button_y = true;
    }

    /// Handle L button press
    pub fn button_l_pressed(&mut self) {
        self.key_input.button_l = true;
    }

    /// Handle R button press
    pub fn button_r_pressed(&mut self) {
        self.key_input.button_r = true;
    }

    /// Handle right button press
    pub fn button_right_pressed(&mut self) {
        self.key_input.right = true;
    }

    /* ===== Button release handling (public) ===== */

    /// Handle up button release
    pub fn button_up_released(&mut self) {
        self.key_input.up = false;
    }

    /// Handle down button release
    pub fn button_down_released(&mut self) {
        self.key_input.down = false;
    }

    /// Handle left button release
    pub fn button_left_released(&mut self) {
        self.key_input.left = false;
    }

    /// Handle right button release
    pub fn button_right_released(&mut self) {
        self.key_input.right = false;
    }

    /// Handle start button release
    pub fn button_start_released(&mut self) {
        self.key_input.start = false;
    }

    /// Handle select button release
    pub fn button_select_released(&mut self) {
        self.key_input.select = false;
    }

    /// Handle A button release
    pub fn button_a_released(&mut self) {
        self.key_input.button_a = false;
    }

    /// Handle B button release
    pub fn button_b_released(&mut self) {
        self.key_input.button_b = false;
    }

    /// Handle X button release
    pub fn button_x_released(&mut self) {
        self.ext_key_in.button_x = false;
    }

    /// Handle Y button release
    pub fn button_y_released(&mut self) {
        self.ext_key_in.button_y = false;
    }

    /// Handle L button release
    pub fn button_l_released(&mut self) {
        self.key_input.button_l = false;
    }

    /// Handle R button release
    pub fn button_r_released(&mut self) {
        self.key_input.button_r = false;
    }
}

#[cfg(test)]
mod tests {
    use crate::firmware::Firmware;

    use super::*;

    #[test]
    fn test_initialize_emulator() {
        // Initialize the Gpu3D struct with basic values
        let emu = Box::new(Emulator::new());
        dbg!(emu.bios_prot);
        assert_eq!(emu.bios_prot, 0);
    }

    #[test]
    fn test_emulator() {
        // Run main emulator
        let mut emu = Box::new(Emulator::new());
        emu.run();
    }
}
