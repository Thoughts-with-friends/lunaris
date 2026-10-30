// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! emulator.hpp
//!
//! Core emulator system that manages CPU, memory, and all peripheral devices
//! Handles the dual-CPU architecture of the Nintendo DS and system timing
mod button;
mod cartridge;
pub mod cheats;
mod dma;
pub mod emu_config;
mod gpu;
mod interrupt;
mod load;
mod read;
mod runner;
mod sound;
pub mod state;
mod timers;
mod write;

use ds_cartridge::NDSCart;
use ds_cpu::{
    arm_cpu::{ArmCpu, CpuType},
    coprocessor_15::Cp15,
};
use ds_gpu::{Gpu, display::regs::SchedulerEvent};
use ds_interrupt::InterruptRegs;
use ds_mem_addr::*;
use ds_peripherals::{ipc::Ipc, rtc::RealTimeClock, spi::SPIBus, timers::NDSTiming, wifi::WiFi};
use ds_spu::SPU;
use emu_config::{BiosMem, Config, ExtKeyInReg, KeyInputReg, PowCnt2Reg};

use crate::dma::NDSDma;

/// Core Nintendo DS emulator system
/// Manages dual ARM CPUs, memory, and all peripheral devices
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Emulator {
    /// Config (host settings, kept by [`Self::load_state`])
    #[serde(skip)]
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

    /// Host data, kept by [`Self::load_state`].
    #[serde(skip)]
    pub arm9_bios: BiosMem<BIOS9_SIZE>,
    #[serde(skip)]
    pub arm7_bios: BiosMem<BIOS7_SIZE>,

    /// Scheduling
    pub system_timestamp: u64,
    pub next_event_time: u64,
    pub gpu_event: SchedulerEvent,
    pub dma_event: SchedulerEvent,

    /// ARM7 cycles not yet turned into an SPU output sample.
    pub spu_cycles: u32,

    /// Raw 2D engine registers (A, B) as last written by the ARM9; see
    /// [`gpu::io_2d`].
    #[serde(with = "crate::emulator::state::nested_array")]
    pub io_2d: [[u8; gpu::io_2d::IO_2D_SIZE]; 2],

    /// Action Replay codes run at the start of every frame (see
    /// [`cheats`]).
    #[serde(skip)]
    pub cheats: Vec<Vec<u32>>,

    /// Slot-2 expansion device (host data, kept by [`Self::load_state`]).
    #[serde(skip)]
    pub slot2: crate::slot2::Slot2Bus,

    /// IPCSYNC / IPCFIFO registers and queues for both CPUs
    pub ipc: Ipc,
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

impl Drop for Emulator {
    /// Writes any unsaved backup memory to the `.sav` file on shutdown.
    fn drop(&mut self) {
        if let Err(_err) = self.cart.flush_save() {
            #[cfg(feature = "log")]
            log::error!("{_err}");
        }
    }
}

impl Default for Emulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Emulator {
    /// Creates a powered-off emulator with zeroed memory.
    ///
    /// ```text
    /// from: frontends (gui/egui, gui/tauri emu_thread, headless example)
    /// to:   set `config`, then load_rom() (which calls power_on()) and
    ///       run() once per frame
    /// ```
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
            main_ram: vec![0; 1024 * 1024 * 4], // 4MB
            shared_wram: vec![0; 1024 * 32],    // 32KB
            arm7_wram: vec![0; 1024 * 64],      // 64KB
            arm9_bios: Default::default(),
            arm7_bios: Default::default(),
            system_timestamp: Default::default(),
            next_event_time: Default::default(),
            gpu_event: Default::default(),
            dma_event: Default::default(),
            io_2d: [[0; gpu::io_2d::IO_2D_SIZE]; 2],
            cheats: Vec::new(),
            slot2: Default::default(),
            spu_cycles: 0,
            ipc: Default::default(),
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

    /// The ARM7 or ARM9 core (used by the interpreter and the scheduler).
    pub fn get_cpu(&self, cpu_type: CpuType) -> &ArmCpu {
        match cpu_type {
            CpuType::Arm7 => &self.arm7,
            CpuType::Arm9 => &self.arm9,
        }
    }

    /// Mutable access to the ARM7 or ARM9 core.
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
    #[expect(unused)]
    fn check_fifo7_interrupt(self) {
        unimplemented!("It is not used in C++ and has no definition.");
    }

    /// Check FIFO interrupt for ARM9.
    ///
    /// NOTE: It is not used in C++ and has no definition.
    #[expect(unused)]
    fn check_fifo9_interrupt(self) {
        unimplemented!("It is not used in C++ and has no definition.");
    }

    /// Runs the hardware divider (DIVCNT 0x04000280) on the current operands.
    ///
    /// ```text
    /// from: ARM9 writes to DIVCNT / DIV_NUMER / DIV_DENOM
    ///       (arm9_write_halfword/word, write_div_operand)
    /// to:   DIV_RESULT / DIVREM_RESULT, read back by arm9_read_word
    /// ```
    ///
    /// Results are available immediately, so the busy bit never shows.
    /// Follows GBATEK "DS Maths" and melonDS `NDS::DivDone`: signed
    /// 32/32, 64/32 (mode 1 and 3) or 64/64 division; division by zero
    /// yields quotient = -1 (or +1 for a negative numerator) and remainder =
    /// numerator, and bit 14 flags a zero 64-bit denominator.
    pub fn start_division(&mut self) {
        // DS Maths: https://problemkaputt.de/gbatek.htm#dsmaths
        let mode = self.divcnt & 0x3; // DIVCNT division mode (bits 0-1)
        let num64 = self.div_numer as i64;
        let den64 = self.div_denom as i64;

        let (quot, rem): (i64, i64) = if mode == 0 {
            let num = num64 as i32;
            let den = den64 as i32;

            if den == 0 {
                (if num < 0 { 1 } else { -1 }, i64::from(num))
            } else if num == i32::MIN && den == -1 {
                // Overflow: the 32-bit result is sign-extended oddly.
                (0x8000_0000, 0)
            } else {
                (i64::from(num / den), i64::from(num % den))
            }
        } else {
            let den = if mode == 2 { den64 } else { i64::from(den64 as i32) };

            if den == 0 {
                (if num64 < 0 { 1 } else { -1 }, num64)
            } else if num64 == i64::MIN && den == -1 {
                (i64::MIN, 0)
            } else {
                (num64 / den, num64 % den)
            }
        };

        // In 32-bit mode a zero numerator/denominator produces -1 in the low
        // word only; the upper word still follows the sign.
        let quot = if mode == 0 && (self.div_denom as u32) == 0 {
            let lo = quot as u32;
            let hi: u32 = if (num64 as i32) < 0 { 0xFFFF_FFFF } else { 0 }; // sign-extend the 32-bit numerator
            (u64::from(hi) << 32 | u64::from(lo)) as i64
        } else {
            quot
        };

        self.div_result = quot as u64;
        self.div_remresult = rem as u64;
        if self.div_denom == 0 {
            // DS Maths: https://problemkaputt.de/gbatek.htm#dsmaths
            self.divcnt |= 1 << 14; // DIVCNT bit 14: division by zero
        } else {
            self.divcnt &= !(1 << 14); // DIVCNT bit 14: division by zero (clear)
        }
    }

    /// Writes one 32-bit half of a divider operand and restarts the division
    /// (ARM9 writes to 0x04000290..0x0400029F -> start_division()).
    pub fn write_div_operand(&mut self, address: u32, word: u32) {
        // DS Maths: https://problemkaputt.de/gbatek.htm#dsmaths
        let (field, high) = match address & 0xC {
            // DIV_NUMER / DIV_DENOM word (address bits 2-3)
            0x0 => (&mut self.div_numer, false), // DIV_NUMER low word
            0x4 => (&mut self.div_numer, true),  // DIV_NUMER high word
            0x8 => (&mut self.div_denom, false), // DIV_DENOM low word
            _ => (&mut self.div_denom, true),
        };
        *field = if high {
            (*field & 0xFFFF_FFFF) | (u64::from(word) << 32) // replace the high 32 bits
        } else {
            (*field & !0xFFFF_FFFF) | u64::from(word) // replace the low 32 bits
        };
        self.start_division();
    }

    /// Runs the square root unit (SQRTCNT 0x040002B0): floor(sqrt(param)),
    /// with a 32-bit (bit 0 clear) or 64-bit (bit 0 set) input.
    /// Called from SQRTCNT / SQRT_PARAM writes; the result is read back
    /// from SQRT_RESULT (0x040002B4).
    pub fn start_sqrt(&mut self) {
        let param = if self.sqrtcnt & 1 != 0 {
            self.sqrt_param
        } else {
            // DS Maths: https://problemkaputt.de/gbatek.htm#dsmaths
            self.sqrt_param & 0xFFFF_FFFF // SQRTCNT mode 0: 32-bit parameter
        };
        self.sqrt_result = param.isqrt() as u32;
    }

    /// Writes one 32-bit half of the square root input and restarts it
    /// (ARM9 writes to 0x040002B8..0x040002BF -> start_sqrt()).
    pub fn write_sqrt_param(&mut self, address: u32, word: u32) {
        self.sqrt_param = if address & 4 != 0 {
            (self.sqrt_param & 0xFFFF_FFFF) | (u64::from(word) << 32) // replace the high 32 bits
        } else {
            (self.sqrt_param & !0xFFFF_FFFF) | u64::from(word) // replace the low 32 bits
        };
        self.start_sqrt();
    }

    /* ===== power and run (public) ===== */

    /// Resets every component to its power-on state and, with
    /// `config.direct_boot_enabled`, boots the cartridge directly.
    ///
    /// ```text
    /// from: load_rom() (after the ROM and firmware are loaded)
    ///   |
    ///   v
    /// power_on()
    ///   1. CPUs, CP15, DMA, cartridge, GPU, SPU, timers, RTC reset
    ///   2. add_gpu_event(0, 256 * 6)      first HBlank of line 0
    ///   3. POWCNT2 (speakers on, Wi-Fi off); config.mac_address patched
    ///      into the firmware; wifi.reset() from the firmware header
    ///      (console type 0x1D, RF chip 0x40, RF channel tables)
    ///   4. scheduler, IPC, 2D I/O copies, keys, IE/IF/IME cleared
    ///   5. direct_boot()                  (skips BIOS/firmware boot code)
    ///   |
    ///   v
    /// to:   frontend calls run() every frame
    /// ```
    pub fn power_on(&mut self) {
        let _ = &mut self.config.bg_enable.fill(true);
        self.cycle_count = 0;
        self.arm9.power_on();
        self.arm7.power_on();
        self.arm9_cp15.power_on();
        self.dma.power_on();
        self.cart.power_on();

        self.gpu.power_on();
        self.add_gpu_event(0, 256 * 6);

        self.spu.power_on();
        self.nds_timing.power_on();
        self.rtc.init();

        self.total_timestamp = 20; //Give the processors some time to run
        self.pow_cnt2.speakers = true;
        self.pow_cnt2.wifi = false;

        if let Some(mac) = self.config.mac_address {
            self.spi.firmware.set_mac_address(mac);
        }
        self.wifi.reset(&self.spi.firmware);
        self.wifi.set_power_cnt(false);

        self.system_timestamp = 0;
        self.next_event_time = 0xffffffff; // no event scheduled
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

        self.ipc.reset();
        self.io_2d = [[0; gpu::io_2d::IO_2D_SIZE]; 2];

        // self.main_ram.clear();
        // self.shared_wram.clear();
        // self.arm7_wram.clear();

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

    /// Writes any pending save data to the `.sav` file right away.
    ///
    /// Saves are also flushed automatically a moment after the game stops
    /// writing, and when the emulator is dropped.
    ///
    /// # Errors
    pub fn flush_save(&mut self) -> Result<(), ds_cartridge::CartridgeError> {
        self.cart.flush_save()
    }

    /// Replaces the save memory with a save file's contents (`.sav`, or a
    /// DeSmuME `.dsv`); written to the `.sav` at the next flush.
    pub fn import_save(&mut self, file: &[u8]) {
        self.cart.backup.import(file);
    }

    /// The save memory as a raw `.sav` image.
    pub fn export_save(&self) -> Vec<u8> {
        self.cart.backup.data().to_vec()
    }

    /// Connects the Wi-Fi controller to other consoles (local wireless /
    /// LAN play, see `ds_net::NetLink`); `None` leaves nobody in range.
    /// Kept across `load_rom` / `power_on`.
    ///
    /// ```text
    /// from: frontend, after Emulator::new()
    /// to:   wifi.set_link -> link.begin() once the game powers Wi-Fi on
    /// ```
    pub fn set_mp_link(&mut self, link: Option<Box<dyn ds_peripherals::wifi::MpLink>>) {
        self.wifi.set_link(link);
    }

    /// Sets the machine up as the firmware leaves it before jumping to the
    /// game, without running BIOS or firmware code (melonDS
    /// `NDS::SetupDirectBoot`).
    ///
    /// ```text
    /// from: power_on() when config.direct_boot_enabled
    ///   |
    ///   v
    /// direct_boot()
    ///   1. copy cartridge header 0x000..0x16F to 0x027FFE00
    ///   2. read ARM9/ARM7 ROM offset, entry point, RAM address, size
    ///      from the copied header (0x027FFE20..)
    ///   3. CPUs: registers/modes after boot, pc = entry points; CP15
    ///      (TCM, high vectors)
    ///   4. boot variables in main RAM: chip ID (0x027FF800/0x027FFC00),
    ///      header CRC / secure area CRC, boot flags
    ///   5. POSTFLG, RCNT, EXMEMCNT, AUXSPICNT, BIOS protection, WRAMCNT=3
    ///   6. copy the ARM9 and ARM7 binaries from ROM into RAM
    ///   7. firmware_direct_boot()          user settings into RAM
    ///   |
    ///   v
    /// to:   first run() starts executing at the entry points
    /// ```
    pub fn direct_boot(&mut self) {
        let mut boot_info = [0_u32; 8];
        // Only the first 0x170 header bytes are copied, like the firmware and
        // melonDS `NDS::SetupDirectBoot`. 0x027FFF70..0x027FFFFF holds the
        // SDK's shared lock words (e.g. the cartridge lock at 0x027FFFE8);
        // filling it with the DSi header extension of hybrid ROMs leaves
        // those locks permanently "held".
        for address in (0..0x170).step_by(4) {
            let word = self.cart.direct_read_word(address);
            // BIOS RAM Usage: https://problemkaputt.de/gbatek.htm#biosramusage
            let offset = ((0x027F_FE00 + address) & MAIN_RAM_MASK) as usize; // NDS cart header copy at 27FFE00h
            self.main_ram[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }

        // msh
        for (i, info) in boot_info.iter_mut().enumerate() {
            *info = self.arm9_read_word((0x27FFE20 + i * 4) as u32);
            #[cfg(feature = "log")]
            log::trace!("Read boot header word {i}: 0x{info:08X}");
        }

        // [BOOT INFO] Examples:
        // boot_info[0] = 0x0004000: ROM offset for ARM9 code
        // boot_info[1] = 0x2000800: ARM9 Entry Point
        // boot_info[2] = 0x2000000: RAM offset for ARM9 code
        // boot_info[3] = 0x00636EC: ARM9 code size
        //
        // boot_info[4] = 0x01DF800: ROM offset for ARM7 code
        // boot_info[5] = 0x2380000: ARM7 Entry Point
        // boot_info[6] = 0x2380000: RAM offset for ARM7 code
        // boot_info[7] = 0x002642C: ARM7 code size
        #[cfg(feature = "log")]
        log::trace!("boot_info={boot_info:#08X?}");

        // Initialize CPUs and regs to after-boot values
        self.arm9.direct_boot(boot_info[1]);
        self.arm7.direct_boot(boot_info[5]);
        self.arm9_cp15.direct_boot();
        self.arm9.set_high_vectors(self.arm9_cp15.high_vectors());

        // Write the ROM chip-id into main RAM
        // NDS BIOS RAM Usage: https://problemkaputt.de/gbatek.htm#:~:text=NDS%20BIOS%20RAM,and%20DSi%20mode).
        let chip_id = self.cart.chip_id();
        self.arm7_write_word(0x027FF800, chip_id); // NDS Gamecart Chip ID 1
        self.arm7_write_word(0x027FF804, chip_id); // NDS Gamecart Chip ID 2
        self.arm7_write_word(0x027FFC00, chip_id); // NDS Gamecart Chip ID 1 (copy of 27FF800h)
        self.arm7_write_word(0x027FFC04, chip_id); // NDS Gamecart Chip ID 2 (copy of 27FF804h)

        // other bullshit
        self.arm7_write_halfword(0x027FF808, self.cart.direct_read_halfword(0x015E)); // Header CRC
        self.arm7_write_halfword(0x027FF80A, self.cart.direct_read_halfword(0x006C)); // Secure Area CRC
        self.arm7_write_halfword(0x027FF850, 0x5835); // NDS7 BIOS CRC (5835h)

        self.arm7_write_halfword(0x027FFC08, self.cart.direct_read_halfword(0x015E)); // Header CRC
        self.arm7_write_halfword(0x027FFC0A, self.cart.direct_read_halfword(0x006C)); // Secure Area CRC

        self.arm7_write_halfword(0x027FFC10, 0x5835); // NDS7 BIOS CRC
        self.arm7_write_halfword(0x027FFC30, 0xFFFF); // GBA Cartridge Header[BEh]
        self.arm7_write_halfword(0x027FFC40, 0x0001); // GBA Cartridge Header[B5h..B7h]

        self.postflg7 = 1;
        self.postflg9 = 1;
        // DS I/O Maps: https://problemkaputt.de/gbatek.htm#dsiomaps
        self.r_cnt = 0x8000; // Debug RCNT: post-boot value

        // Post-boot EXMEMCNT and AUXSPICNT (melonDS `NDS::SetupDirectBoot`).
        self.ex_mem_cnt = 0xE880;
        // DS Cartridge I/O Ports: https://problemkaputt.de/gbatek.htm#dscartridgeioports
        self.cart.set_auxspicnt(0x8000); // AUXSPICNT bit 15: NDS slot enable

        // DS Memory Control - BIOS: https://problemkaputt.de/gbatek.htm#dsmemorycontrolbios
        self.bios_prot = 0x1204; // BIOSPROT: ARM7 BIOS protected from 1204h
        self.wram_cnt = 3;

        // Load ROM into Main RAM for ARM9
        let mut i = 0;
        while i < boot_info[3] {
            // for (i, _) in (0..boot_info[3]).enumerate().step_by(4) {
            // for i in (0..boot_info[3]).step_by(4) {
            let rom_data: u32 = self.cart.direct_read_word(boot_info[0] + i);
            #[cfg(feature = "log")]
            log::trace!(
                "ROM_READ [ARM9_BOOT] Addr: 0x{:08X} Value: 0x{rom_data:08X}",
                boot_info[0] + i
            );

            self.arm9_write_word(boot_info[2] + i, rom_data);
            #[cfg(feature = "log")]
            log::trace!(
                "ROM_WRITE [ARM9_BOOT] Addr: 0x{:08X} Word: 0x{rom_data:08X}",
                boot_info[2] + i
            );

            i += 4;
        }

        // Load ROM into Main RAM for ARM7
        let mut i = 0;
        while i < boot_info[7] {
            let rom_data: u32 = self.cart.direct_read_word(boot_info[4] + i);

            #[cfg(feature = "log")]
            log::trace!(
                "ROM_READ [ARM7_BOOT] Addr: 0x{:08X} Word: 0x{rom_data:08X}",
                boot_info[4] + i
            );
            self.arm7_write_word(boot_info[6] + i, rom_data);
            i += 4;
        }

        self.firmware_direct_boot();
        self.cycles = 0;
    }

    /// Writes the firmware values the boot menu leaves in main RAM:
    /// firmware size, part of the header, and the user settings (name,
    /// language, touch calibration...) at 0x027FFC80.
    ///
    /// ```text
    /// from: direct_boot() step 7
    /// to:   read by the game's OS_GetOwnerInfo / TP calibration at boot
    /// ```
    ///
    fn firmware_direct_boot(&mut self) {
        // Wifi FLASH User Settings Bad (0=Okay, 1=Bad)
        self.arm7_write_word(0x27FF864, 0);

        // Wifi FLASH User Settings FLASH Address
        let data = self.spi.firmware.read_u16(0x20) << 3;
        // BIOS RAM Usage: https://problemkaputt.de/gbatek.htm#biosramusage
        self.arm7_write_word(0x27FF868, data as u32); // Wifi FLASH User Settings FLASH Address (fmw[20h]*8)

        // Wifi FLASH firmware part5 crc16 (359Ah) (fmw[026h])
        let data = self.spi.firmware.read_u16(0x26);
        self.arm7_write_halfword(0x027FF874, data); // Wifi FLASH firmware part5 crc16 (fmw[026h])

        // Wifi FLASH firmware part3/part4 crc16 (fmw[004h] or ZERO)
        let data = self.spi.firmware.read_u16(0x04);
        self.arm7_write_halfword(0x027FF876, data); // Wifi FLASH firmware part3/part4 crc16 (fmw[004h])

        for i in (0..0x70).step_by(4) {
            // 70h bytes of firmware user settings
            let user_data = self.spi.firmware.user_data;
            let index = (user_data + i) as usize;
            let data = self.spi.firmware.read_u32(index);
            self.arm7_write_word((0x027FFC80 + i) as u32, data); // Wifi FLASH User Settings at 27FFC80h
        }
    }

    /// Toggles `config.test` and logs IE/IF of both CPUs (debug aid, `log`
    /// feature only).
    pub fn debug(&mut self) {
        #[cfg(feature = "log")]
        {
            //arm7.set_disassembly(!arm7.can_disassemble());
            //arm9.print_info();
            self.config.test = !self.config.test;
            log::trace!(
                "IE9: {:08X} IF9: {:08X}",
                self.int9_reg.irq_enable,
                self.int9_reg.irq_flags
            );
            log::trace!(
                "IE7: {:08X} IF7: {:08X}",
                self.int7_reg.irq_enable,
                self.int7_reg.irq_flags
            );
        }
    }

    /// Whether any enabled interrupt is requested (IE & IF), ignoring IME.
    /// This is what releases a halted CPU (execute() step 2).
    pub const fn irq_line_raised(&self, cpu_id: i32) -> bool {
        match cpu_id {
            0 => (self.int9_reg.irq_enable & self.int9_reg.irq_flags) != 0,
            _ => (self.int7_reg.irq_enable & self.int7_reg.irq_flags) != 0,
        }
    }

    /// Whether the CPU should take an IRQ exception: IE & IF != 0 and IME
    /// set (CPSR.I is checked by the caller, execute()).
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
    ///
    /// Must track `system_timestamp`, not the unused `total_timestamp` field
    /// (a dead C++ struct member kept only for layout parity). Returning a
    /// frozen value here makes halted CPUs rewind their local timestamp
    /// instead of catching up to the scheduler, which hangs the run loop
    /// forever on the first HALT/DMA wait.
    pub fn get_timestamp(&self) -> u64 {
        self.system_timestamp
    }

    /* ===== get frame(public) ===== */

    /// Copies the finished top screen (256x192 0x00RRGGBB) into `buffer`.
    /// Called by the frontends after run().
    pub fn get_upper_frame(&self, buffer: &mut [u32]) {
        self.gpu.get_upper_frame(buffer);
    }

    /// Copies the finished bottom (touch) screen into `buffer`.
    /// Called by the frontends after run().
    pub fn get_lower_frame(&self, buffer: &mut [u32]) {
        self.gpu.get_lower_frame(buffer);
    }

    /// Replaces the top screen buffer (unused by the frontends).
    pub fn set_upper_screen(&mut self, buffer: Vec<u32>) {
        self.gpu.set_upper_buffer(buffer);
    }

    /// Replaces the bottom screen buffer (unused by the frontends).
    pub fn set_lower_screen(&mut self, buffer: Vec<u32>) {
        self.gpu.set_lower_buffer(buffer);
    }

    /* frame, display and dma ===== (public) ===== */

    /// Not implemented (CorgiDS API placeholder); use `gpu.is_frame_complete()`.
    pub fn frame_complete(&self) -> bool {
        unimplemented!();
    }

    /// Not implemented (CorgiDS API placeholder); POWCNT1 bit 15 decides
    /// which engine drives the top screen.
    pub fn display_swapped(&self) -> bool {
        unimplemented!();
    }

    /// Whether any DMA channel is transferring; execute() stalls both CPUs
    /// while it is.
    pub fn dma_active(&self) -> bool {
        self.dma.is_active()
    }

    /* ===== DMA (public)  ===== */

    // inlined these functions.
    // pub fn hblank_dma_request(&mut self);
    // pub fn gamecart_dma_request(&mut self);
    // pub fn gxfifo_dma_request(&mut self);

    /// HBlank DMA trigger: gpu_handle_event() -> hblank_request().
    pub fn hblank_dma_request(&mut self) {
        self.hblank_request(); // inline
    }

    /// Cartridge DMA trigger: cartridge_run() -> gamecart_request().
    pub fn gamecart_dma_request(&mut self) {
        self.gamecart_request(); // wrapping
    }

    /// GXFIFO DMA trigger: check_fifo_dma() & co. -> gxfifo_request().
    // GPU::check_GXFIFO_DMA() -> eng_3D.check_FIFO_DMA()
    pub fn gxfifo_dma_request(&mut self) {
        self.gxfifo_request(); // wrapping
    }

    /// Starts GXFIFO DMAs if the FIFO is less than half full; called by
    /// dma_write_cnt() when a timing-7 channel is enabled.
    // GPU::check_GXFIFO_DMA() -> eng_3D.check_FIFO_DMA()
    pub fn check_gxfifo_dma(&mut self) {
        if self.gpu.engine_3d.gxfifo.len() < 128 {
            self.gxfifo_dma_request();
        }
    }

    /* ===== add and sys stamp (public) ===== */

    /// Schedules the GPU scanline event `relative_time` cycles from now
    /// (power_on() only; later events use reschedule_gpu_event()).
    pub fn add_gpu_event(&mut self, event_id: i32, relative_time: u64) {
        self.gpu_event.id = event_id;
        self.gpu_event.activation_time = self.system_timestamp + relative_time;

        if self.gpu_event.activation_time < self.next_event_time {
            self.next_event_time = self.gpu_event.activation_time;
        }
    }

    /// Schedules the next GPU (scanline) event `relative_time` cycles after
    /// the one being handled (called at the end of gpu_handle_event()). Basing it on the due time
    /// rather than the
    /// current time keeps handling latency from accumulating: frames used
    /// to run ~2% long (572k instead of 560190 cycles), which made the SPU
    /// produce ~33.4 kHz instead of ~32.7 kHz of audio per real second.
    pub fn reschedule_gpu_event(&mut self, event_id: i32, relative_time: u64) {
        self.gpu_event.id = event_id;
        self.gpu_event.activation_time += relative_time;
    }

    /// Schedules DMA channel `event_id` to run `relative_time` cycles from
    /// now; run() then calls dma_handle_event(). Only one DMA event is
    /// pending at a time (the last one scheduled wins).
    pub fn add_dma_event(&mut self, event_id: i32, relative_time: u64) {
        #[cfg(feature = "log")]
        log::trace!("call add_dma_event");

        self.dma_event.id = event_id;
        self.dma_event.processing = true;
        self.dma_event.activation_time = self.system_timestamp + relative_time;

        if self.dma_event.activation_time < self.next_event_time {
            self.next_event_time = self.dma_event.activation_time;
        }
    }

    /// Advances `system_timestamp` to the end of the next time slice.
    ///
    /// ```text
    /// from: run() step 2a
    ///   |
    ///   v
    /// calculate_system_timestamp()
    ///   1. next = earliest pending event (GPU, and DMA if scheduled)
    ///   2. slice = next - now, clamped to 1..=64 cycles
    ///      (an overdue event also gives a 64-cycle slice)
    ///   3. system_timestamp += slice
    ///   |
    ///   v
    /// to:   run() lets both CPUs catch up to the new time
    /// ```
    pub fn calculate_system_timestamp(&mut self) {
        // NOTE: It appears that next_event_time < system_timestamp is in accordance with CorgiDS specifications.
        // (This is counterintuitive and the reason is unclear.)

        // Advance the scheduler to the next event, but at most
        // `SLICE_CYCLES` bus cycles so that CPU interleaving (IPC, IRQ
        // latency) stays fine-grained. A larger slice = fewer loop
        // iterations = faster emulation (CorgiDS used 20).
        // const SLICE_CYCLES: i32 = 64;
        const SLICE_CYCLES: i32 = 64;

        // Next pending event (`next_event_time` alone only ever decreased,
        // so it stayed in the past after the first event).
        let mut next = self.gpu_event.activation_time;
        if self.dma_event.processing {
            next = next.min(self.dma_event.activation_time);
        }
        self.next_event_time = next;

        let cycles = self.next_event_time.wrapping_sub(self.system_timestamp) as i32;
        self.system_timestamp += match cycles {
            1..=SLICE_CYCLES => cycles as u64,
            _ => SLICE_CYCLES as u64,
        };
    }

    /* ===== touchscreen (public) ===== */

    /// Sets the stylus position for the next frames: pixel `x, y` on the
    /// bottom screen, or `y == 0xFFF` for "not touching".
    ///
    /// ```text
    /// from: frontend before run() (gui/egui input.rs Touch::apply,
    ///       gui/tauri emu_thread apply_touch, headless TOUCH=)
    ///   |
    ///   v
    /// touchscreen_press(x, y)
    ///   1. EXTKEYIN bit 6 (pen down, read by the ARM7 at 0x04000136)
    ///   2. spi.touchscreen_press() -> TSC latches ADC x<<4, y<<4
    ///   |
    ///   v
    /// to:   ARM7 TP code samples the TSC over SPI (write_spidata, dev 2)
    ///       and passes the result to the ARM9 (IPC FIFO tag 6 / auto
    ///       sampling buffer)
    /// ```
    pub fn touchscreen_press(&mut self, x: i32, y: i32) {
        // DS Touch Screen Controller (TSC): https://problemkaputt.de/gbatek.htm#dstouchscreencontrollertsc
        self.ext_key_in.pen_down = y != 0xfff; // Y = FFFh: pen not touching the screen
        self.spi.touchscreen_press(x, y);
    }

    // ===== read and write arm =====
    // - moved read_arm7/read_arm9, write_arm7/write_arm9
    /* ===== cartrige ===== */
    /// Copies the KEY1 seed table (ARM7 BIOS 0x30..0x1077) into `buffer`
    /// (CorgiDS API; the cartridge crate now reads the BIOS itself).
    pub fn cart_copy_keybuffer(&self, buffer: &mut [u8]) {
        // DS Encryption by Gamecode/Idcode (KEY1): https://problemkaputt.de/gbatek.htm#dsencryptionbygamecodeidcodekey1
        if let Some(bios) = &self.arm7_bios.get(0x30..0x30 + 0x1048) {
            // KEY1 table: 1048h bytes at ARM7 BIOS offset 30h
            buffer[..0x1048].copy_from_slice(bios); // KEY1 table size
        } else {
            #[cfg(feature = "log")]
            log::warn!("arm7_bios need: 0x30..0x30 + 0x1048"); // KEY1 table location in the ARM7 BIOS
        }
    }

    /// Writes a halfword of the cartridge header copy at 0x027FFE00
    /// (CorgiDS API; unused).
    pub fn cart_write_header(&mut self, address: u32, halfword: u16) {
        // BIOS RAM Usage: https://problemkaputt.de/gbatek.htm#biosramusage
        let addr = ((0x027FFE00 + (address & 0x1FF)) & MAIN_RAM_MASK) as usize; // NDS cart header copy at 27FFE00h (1FFh bytes)

        if addr + 1 < self.main_ram.len() {
            let bytes = halfword.to_le_bytes();
            self.main_ram[addr] = bytes[0];
            self.main_ram[addr + 1] = bytes[1];
        }
    }
}
