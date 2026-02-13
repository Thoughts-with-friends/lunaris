// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
//! Core emulator system that manages CPU, memory, and all peripheral devices
//! Handles the dual-CPU architecture of the Nintendo DS and system timing
mod button;
mod cartridge;
mod dma;
pub mod emu_config;
mod interrupt;
mod load;
mod read;
mod read_arm7;
mod read_arm9;
mod runner;
mod timers;
mod write;
mod write_arm7;
mod write_arm9;

use crate::cpu::arm_cpu::ArmCpu;
use crate::cpu::coprocessor_15::Cp15;
use lunaris_ds_audio::SPU;
use lunaris_ds_gpu::gpu_root::{Gpu, register::SchedulerEvent};
use lunaris_ds_mem_const::*;
use std::collections::VecDeque;

use crate::cartridge::NDSCart;
use crate::cpu::arm_cpu::CpuType;
use crate::dma::NDSDma;
use crate::interrupts::InterruptRegs;
use crate::ipc::{IpcFifo, IpcSync};
use crate::rtc::RealTimeClock;
use crate::spi::SPIBus;
use crate::timers::NDSTiming;
use crate::wifi::WiFi;
use emu_config::{BiosMem, Config, ExtKeyInReg, KeyInputReg, PowCnt2Reg};

/// Core Nintendo DS emulator system
/// Manages dual ARM CPUs, memory, and all peripheral devices
#[derive(Debug)]
pub struct Emulator {
    /// Config
    pub config: Config,

    pub cycle_count: u64,
    pub arm7: ArmCpu,
    pub arm9: ArmCpu,
    pub arm9_cp15: Cp15,
    pub cart: NDSCart,
    pub dma: NDSDma,
    pub gpu: Gpu,
    pub rtc: RealTimeClock,
    pub spi: SPIBus,
    pub spu: SPU,
    pub nds_timing: NDSTiming,
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

impl Default for Emulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Emulator {
    /// Create a new emulator instance with default values
    pub fn new() -> Self {
        Self {
            arm7: ArmCpu::new(1, CpuType::Arm7),
            arm9: ArmCpu::new(0, CpuType::Arm9),
            config: Default::default(),
            cycle_count: Default::default(),
            arm9_cp15: Default::default(),
            cart: Default::default(),
            dma: Default::default(),
            gpu: Default::default(),
            rtc: Default::default(),
            spi: Default::default(),
            spu: Default::default(),
            nds_timing: Default::default(),
            wifi: Default::default(),
            main_ram: Default::default(),
            shared_wram: Default::default(),
            arm7_wram: Default::default(),
            arm9_bios: Default::default(),
            arm7_bios: Default::default(),
            system_timestamp: Default::default(),
            next_event_time: Default::default(),
            gpu_event: Default::default(),
            dma_event: Default::default(),
            ipc_sync_nds9: Default::default(),
            ipc_sync_nds7: Default::default(),
            fifo7: Default::default(),
            fifo9: Default::default(),
            fifo7_queue: Default::default(),
            fifo9_queue: Default::default(),
            aux_spi_cnt: Default::default(),
            int7_reg: Default::default(),
            int9_reg: Default::default(),
            key_input: Default::default(),
            ext_key_in: Default::default(),
            pow_cnt2: Default::default(),
            dma_fill: Default::default(),
            sio_cnt: Default::default(),
            r_cnt: Default::default(),
            ex_mem_cnt: Default::default(),
            wram_cnt: Default::default(),
            divcnt: Default::default(),
            div_numer: Default::default(),
            div_denom: Default::default(),
            div_result: Default::default(),
            div_remresult: Default::default(),
            sqrtcnt: Default::default(),
            sqrt_result: Default::default(),
            sqrt_param: Default::default(),
            postflg7: Default::default(),
            postflg9: Default::default(),
            bios_prot: Default::default(),
            hstep_even: Default::default(),
            cycles: Default::default(),
            total_timestamp: Default::default(),
            last_arm9_timestamp: Default::default(),
            last_arm7_timestamp: Default::default(),
        }
    }

    pub fn get_cpu(&self, cpu_type: CpuType) -> &ArmCpu {
        match cpu_type {
            CpuType::Arm7 => &self.arm7,
            CpuType::Arm9 => &self.arm9,
        }
    }

    pub fn get_cpu_mut(&mut self, cpu_type: CpuType) -> &mut ArmCpu {
        match cpu_type {
            CpuType::Arm7 => &mut self.arm7,
            CpuType::Arm9 => &mut self.arm9,
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
        for bg in &mut self.config.bg_enable {
            *bg = true;
        }
        self.cycle_count = 0;
        self.arm9.power_on();
        self.arm7.power_on();
        self.arm9_cp15.power_on();
        self.dma.power_on();
        self.gpu.power_on();
        self.spu.power_on();
        self.nds_timing.power_on();
        self.rtc.init();
        self.total_timestamp = 20; //Give the processors some time to run
        self.pow_cnt2.speakers = true;
        self.pow_cnt2.wifi = false;

        self.system_timestamp = 0;
        self.next_event_time = 0xffffffff;
        self.gpu_event.activation_time = 0;
        self.gpu_event.id = 0;
        self.dma_event.activation_time = 0;
        self.dma_event.processing = false;
        self.dma_event.id = 0;

        self.postflg7 = 0;
        self.postflg9 = 0;
        self.aux_spi_cnt = 0;
        self.sio_cnt = 0;
        self.bios_prot = 0;
        self.hstep_even = true;

        self.sqrtcnt = 0;
        self.divcnt = 0;

        self.key_input.button_a = false;
        self.key_input.button_b = false;
        self.key_input.select = false;
        self.key_input.start = false;
        self.key_input.right = false;
        self.key_input.left = false;
        self.key_input.up = false;
        self.key_input.down = false;
        self.key_input.button_r = false;
        self.key_input.button_l = false;

        self.ext_key_in.button_x = false;
        self.ext_key_in.button_y = false;
        self.ext_key_in.pen_down = false;
        self.ext_key_in.hinge_closed = false;

        self.ipc_sync_nds7.input = 0;
        self.ipc_sync_nds9.input = 0;
        self.fifo7.write_cnt(0);
        self.fifo7.error = false;
        self.fifo9.write_cnt(0);
        self.fifo9.error = false;
        self.fifo7.recent_word = 0;
        self.fifo9.recent_word = 0;

        self.main_ram.clear();
        self.shared_wram.clear();
        self.arm7_wram.clear();

        self.int7_reg.ime = 0;
        self.int7_reg.irq_enable = 0;
        self.int7_reg.irq_flags = 0;
        self.int9_reg.ime = 0;
        self.int9_reg.irq_enable = 0;
        self.int9_reg.irq_flags = 0;

        if self.config.direct_boot_enabled {
            self.direct_boot();
        }
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
        #[cfg(feature = "tracing")]
        {
            //arm7.set_disassembly(!arm7.can_disassemble());
            //arm9.print_info();
            self.config.test = !self.config.test;
            tracing::debug!(
                "IE9: {:08X} IF9: {:08X}",
                self.int9_reg.irq_enable,
                self.int9_reg.irq_flags
            );
            tracing::debug!(
                "IE7: {:08X} IF7: {:08X}",
                self.int7_reg.irq_enable,
                self.int7_reg.irq_flags
            );
        }
    }

    /// Check whether a CPU is requesting an interrupt.
    pub fn requesting_interrupt(&self, cpu_id: i32) -> bool {
        match cpu_id {
            0 => {
                (self.int9_reg.irq_enable & self.int9_reg.irq_flags) != 0 && self.int9_reg.ime != 0
            }
            _ => {
                (self.int7_reg.irq_enable & self.int7_reg.irq_flags) != 0 && self.int7_reg.ime != 0
            }
        }
    }

    /// Get current system timestamp.
    pub fn get_timestamp(&self) -> u64 {
        self.total_timestamp
    }

    /* ===== get frame(public) ===== */

    /// Copy upper screen framebuffer.
    pub fn get_upper_frame(&self, buffer: &mut [u32]) {
        self.gpu.get_upper_frame(buffer);
    }

    /// Copy lower screen framebuffer.
    pub fn get_lower_frame(&self, buffer: &mut [u32]) {
        self.gpu.get_lower_frame(buffer);
    }

    /// Set upper screen framebuffer.
    pub fn set_upper_screen(&mut self, buffer: Vec<u32>) {
        self.gpu.set_upper_buffer(buffer);
    }

    /// Set lower screen framebuffer.
    pub fn set_lower_screen(&mut self, buffer: Vec<u32>) {
        self.gpu.set_lower_buffer(buffer);
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
    pub fn dma_active(&self) -> bool {
        self.dma.is_active()
    }

    /* ===== DMA (public)  ===== */

    // inlined these functions.
    // pub fn hblank_dma_request(&mut self);
    // pub fn gamecart_dma_request(&mut self);
    // pub fn gxfifo_dma_request(&mut self);

    /// Request HBLANK DMA.
    pub fn hblank_dma_request(&mut self) {
        self.hblank_request() // inline
    }

    /// Request game cartridge DMA.
    pub fn gamecart_dma_request(&mut self) {
        self.gamecart_request() // wrapping
    }

    /// Request GX FIFO DMA.
    pub fn gxfifo_dma_request(&mut self) {
        self.gfxfifo_request(); // wrapping
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
            0 => self.swi9(),
            _ => self.swi7(),
        }
    }

    // ===== read and write arm =====
    // - moved read_arm7/read_arm9, write_arm7/write_arm9

    /* ===== cartrige ===== */

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
}
