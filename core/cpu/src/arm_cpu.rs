//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! cpuinsters.hpp
//!

/// General-purpose register indices
pub const REG_SP: u32 = 13;
pub const REG_LR: u32 = 14;
pub const REG_PC: u32 = 15;

/// Carry out of `a + b`.
#[inline]
pub const fn carry_add(a: u32, b: u32) -> bool {
    // ARM Opcodes: Data Processing (ALU): https://problemkaputt.de/gbatek.htm#armopcodesdataprocessingalu
    (0xFFFF_FFFF_u32.wrapping_sub(a)) < b // carry out of an unsigned 32-bit add
}

/// Signed overflow of `a + b = result`.
#[inline]
pub const fn add_overflow(a: u32, b: u32, result: u32) -> bool {
    (((a ^ b) & 0x8000_0000) == 0) && (((a ^ result) & 0x8000_0000) != 0) // signed overflow: operands share a sign (bit 31) but the result differs
}

/// Signed overflow of `a - b = result`.
#[inline]
pub const fn sub_overflow(a: u32, b: u32, result: u32) -> bool {
    (((a ^ b) & 0x8000_0000) != 0) && (((a ^ result) & 0x8000_0000) != 0) // signed overflow: operand signs differ and the result sign differs from a
}

/// ARM processor modes (CPSR.M)
#[derive(serde::Serialize, serde::Deserialize, Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum PsrMode {
    /// User mode
    #[default]
    // ARM CPU Flags & Condition Field (cond): https://problemkaputt.de/gbatek.htm#armcpuflagsconditionfieldcond
    User = 0x10, // M4-M0 = 10000b
    /// FIQ (Fast Interrupt) mode
    Fiq = 0x11, // M4-M0 = 10001b
    /// IRQ (Interrupt) mode
    Irq = 0x12, // M4-M0 = 10010b
    /// Supervisor mode
    Supervisor = 0x13, // M4-M0 = 10011b
    /// Abort mode
    Abort = 0x17, // M4-M0 = 10111b
    /// Undefined instruction mode
    Undefined = 0x1B, // M4-M0 = 11011b
    /// System mode
    System = 0x1F, // M4-M0 = 11111b
}

impl PsrMode {
    /// Const-safe conversion from raw CPSR.M value
    pub const fn from_u32(value: u32) -> Option<Self> {
        // 2147483711
        // 0b11101 = 0x1d
        // 0b11111 = 0x1f
        Some(match value {
            0x10 => Self::User,       // User
            0x11 => Self::Fiq,        // FIQ
            0x12 => Self::Irq,        // IRQ
            0x13 => Self::Supervisor, // Supervisor
            0x17 => Self::Abort,      // Abort
            0x1B => Self::Undefined,  // Undefined
            0x1F => Self::System,     // System
            _ => return None,
        })
    }
}

/// Program Status Register flags
#[derive(serde::Serialize, serde::Deserialize, Copy, Clone, Debug, Default)]
pub struct PsrFlags {
    /// Negative flag (result was negative)
    pub negative: bool,
    /// Zero flag (result was zero)
    pub zero: bool,
    /// Carry flag (carry out or borrow)
    pub carry: bool,
    /// Overflow flag (signed arithmetic overflow)
    pub overflow: bool,
    /// Sticky overflow flag
    pub sticky_overflow: bool,

    /// IRQ interrupt disabled
    pub irq_disabled: bool,
    /// FIQ interrupt disabled
    pub fiq_disabled: bool,
    /// Thumb mode enabled
    pub thumb_on: bool,

    /// Current processor mode
    pub mode: PsrMode,
}

impl PsrFlags {
    /// Serialize flags into a 32-bit CPSR/SPSR value
    pub const fn get(&self) -> u32 {
        let mut reg = 0;
        // ARM CPU Flags & Condition Field (cond): https://problemkaputt.de/gbatek.htm#armcpuflagsconditionfieldcond
        reg |= (self.negative as u32) << 31; // N flag (bit 31)
        reg |= (self.zero as u32) << 30; // Z flag (bit 30)
        reg |= (self.carry as u32) << 29; // C flag (bit 29)
        reg |= (self.overflow as u32) << 28; // V flag (bit 28)
        reg |= (self.sticky_overflow as u32) << 27; // Q sticky overflow flag (bit 27)

        reg |= (self.irq_disabled as u32) << 7;
        reg |= (self.fiq_disabled as u32) << 6;
        reg |= (self.thumb_on as u32) << 5;

        reg |= self.mode as u32;

        reg
    }

    /// Load flags from a 32-bit CPSR/SPSR value
    #[allow(clippy::missing_const_for_fn)] // not const: logs under feature "log"
    pub fn set(&mut self, value: u32) {
        // 1610612829
        self.negative = (value & (1 << 31)) != 0;
        self.zero = (value & (1 << 30)) != 0; // Z flag (bit 30)
        self.carry = (value & (1 << 29)) != 0; // C flag (bit 29)
        self.overflow = (value & (1 << 28)) != 0; // V flag (bit 28)
        self.sticky_overflow = (value & (1 << 27)) != 0; // Q sticky overflow flag (bit 27)

        self.irq_disabled = (value & (1 << 7)) != 0; // I bit (7): IRQ disabled
        self.fiq_disabled = (value & (1 << 6)) != 0; // F bit (6): FIQ disabled
        self.thumb_on = (value & (1 << 5)) != 0; // T bit (5): Thumb state

        if let Some(mode) = PsrMode::from_u32(value & 0x1F) {
            // CPSR mode bits M4-M0 (bits 0-4)
            self.mode = mode;
        } else {
            #[cfg(feature = "log")]
            log::warn!("invalid PsrMode: {value}");
        }
    }
}

/// This type exists to avoid cross references between emu and cpu.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default, PartialEq)]
pub enum CpuType {
    #[default]
    Arm7,
    Arm9,
}

/// ARM CPU core
#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
pub struct ArmCpu {
    pub cpu_type: CpuType,

    /// CPU ID (ARM9 / ARM7 etc.)
    /// - Arm7: 1
    /// - Arm9: 0
    pub cpu_id: i32,

    /// Halt state
    pub halted: bool,

    // Banked stack pointers
    /// Stack pointer for supervisor mode
    sp_svc: u32,
    /// Stack pointer for IRQ mode
    sp_irq: u32,
    /// Stack pointer for FIQ mode
    sp_fiq: u32,
    /// Stack pointer for abort mode
    sp_abt: u32,
    /// Stack pointer for undefined mode
    sp_und: u32,

    // Banked link registers
    /// Link register for supervisor mode
    lr_svc: u32,
    /// Link register for IRQ mode
    lr_irq: u32,
    /// Link register for FIQ mode
    lr_fiq: u32,
    /// Link register for abort mode
    lr_abt: u32,
    /// Link register for undefined mode
    lr_und: u32,

    /// FIQ banked registers (r8–r12)
    fiq_regs: [u32; 5],

    /// General-purpose registers r0–r15
    pub regs: [u32; 16],

    /// Current Program Status Register
    pub cpsr: PsrFlags,

    /// Saved Program Status Registers (indexed by mode)
    spsr: [PsrFlags; 0x20], // indexed by the 5-bit mode number

    /// Exception vector base address
    exception_base: u32,

    /// Cycle timestamp
    pub timestamp: u64,
    pub last_timestamp: u64,

    /// Currently executing instruction
    pub current_instr: u32,

    /// Ring buffer of recent branch targets (debugging aid; see
    /// [`Self::recent_pcs`]).
    pc_history: [u32; PC_HISTORY_LEN],
    pc_history_pos: usize,
    /// Address of the most recently executed instruction.
    pub last_pc: u32,

    /// Code waitstates [region][n32/s32/n16/s16]
    ///
    /// Organized by memory region and access type
    /// - Format: [region][access_type]
    /// - Access types: 0=n32, 1=s32, 2=n16, 3=s16
    code_waitstates: [[i32; 4]; 16],

    /// Data waitstates [region][n32/s32/n16/s16]
    ///
    /// Organized by memory region and access type
    /// - Format: [region][access_type]
    /// - Access types: 0=n32, 1=s32, 2=n16, 3=s16
    data_waitstates: [[i32; 4]; 16],
}

/// Number of branch targets kept in [`ArmCpu`]'s PC history.
pub const PC_HISTORY_LEN: usize = 32;

impl ArmCpu {
    /// Records the address of the instruction about to execute; only
    /// non-sequential addresses (branch targets) enter the history.
    #[inline]
    pub const fn record_pc(&mut self, pc: u32) {
        if pc.wrapping_sub(self.last_pc) > 4 {
            self.pc_history[self.pc_history_pos] = pc;
            self.pc_history_pos = (self.pc_history_pos + 1) % PC_HISTORY_LEN;
        }
        self.last_pc = pc;
    }

    /// Recent branch targets, oldest first.
    pub fn recent_pcs(&self) -> impl Iterator<Item = u32> + '_ {
        let (newer, older) = self.pc_history.split_at(self.pc_history_pos);
        older.iter().chain(newer).copied()
    }

    /// Create a new ARM CPU
    pub fn new(cpu_id: i32, cpu_type: CpuType) -> Self {
        let mut code_waitstates: [[i32; 4]; 16] = [[0; 4]; 16];
        let mut data_waitstates: [[i32; 4]; 16] = [[0; 4]; 16];

        // Fill waitstates with dummy values to prevent bugs
        for row in code_waitstates.iter_mut().take(4) {
            for cell in row.iter_mut() {
                *cell = 1;
            }
        }

        // Initialize waitstates based upon GBATek's info
        // Note that 8-bit reads/writes have the same timings as 16 bit
        if cpu_id <= 0 {
            // ARM9 - timings here are doubled to represent 66 MHz
            // Of interest is that the ARM9 is incapable of sequential code fetches
            // Thus ALL code fetches are subject to the three cycle penalty... so much for 66 MHz
            // Index: [address bits 24-27 region][0=N32, 1=S32, 2=N16, 3=S16]; region 2=Main RAM, 3=WRAM, 4=I/O, 5=palette, 6=VRAM, 7=OAM, F=BIOS
            // DS Memory Timings: https://problemkaputt.de/gbatek.htm#dsmemorytimings
            code_waitstates[0x2][0] = 1; // Main RAM N32 code access: 1 extra cycle(s)
            code_waitstates[0x3][0] = 8; // shared/ARM7 WRAM N32 code access: 8 extra cycle(s)
            code_waitstates[0x4][0] = 8; // I/O N32 code access: 8 extra cycle(s)
            code_waitstates[0x5][0] = 10; // palette N32 code access: 10 extra cycle(s)
            code_waitstates[0x6][0] = 10; // VRAM N32 code access: 10 extra cycle(s)
            code_waitstates[0x7][0] = 8; // OAM N32 code access: 8 extra cycle(s)
            code_waitstates[0xF][0] = 8; // BIOS N32 code access: 8 extra cycle(s)

            code_waitstates[0x2][1] = 1; // Main RAM S32 code access: 1 extra cycle(s)
            code_waitstates[0x3][1] = 8; // shared/ARM7 WRAM S32 code access: 8 extra cycle(s)
            code_waitstates[0x4][1] = 8; // I/O S32 code access: 8 extra cycle(s)
            code_waitstates[0x5][1] = 10; // palette S32 code access: 10 extra cycle(s)
            code_waitstates[0x6][1] = 10; // VRAM S32 code access: 10 extra cycle(s)
            code_waitstates[0x7][1] = 8; // OAM S32 code access: 8 extra cycle(s)
            code_waitstates[0xF][1] = 8; // BIOS S32 code access: 8 extra cycle(s)

            code_waitstates[0x2][2] = 1; // Main RAM N16 code access: 1 extra cycle(s)
            code_waitstates[0x3][2] = 4; // shared/ARM7 WRAM N16 code access: 4 extra cycle(s)
            code_waitstates[0x4][2] = 4; // I/O N16 code access: 4 extra cycle(s)
            code_waitstates[0x5][2] = 5; // palette N16 code access: 5 extra cycle(s)
            code_waitstates[0x6][2] = 5; // VRAM N16 code access: 5 extra cycle(s)
            code_waitstates[0x7][2] = 4; // OAM N16 code access: 4 extra cycle(s)
            code_waitstates[0xF][2] = 4; // BIOS N16 code access: 4 extra cycle(s)

            code_waitstates[0x2][3] = 1; // Main RAM S16 code access: 1 extra cycle(s)
            code_waitstates[0x3][3] = 4; // shared/ARM7 WRAM S16 code access: 4 extra cycle(s)
            code_waitstates[0x4][3] = 4; // I/O S16 code access: 4 extra cycle(s)
            code_waitstates[0x5][3] = 5; // palette S16 code access: 5 extra cycle(s)
            code_waitstates[0x6][3] = 5; // VRAM S16 code access: 5 extra cycle(s)
            code_waitstates[0x7][3] = 4; // OAM S16 code access: 4 extra cycle(s)
            code_waitstates[0xF][3] = 4; // BIOS S16 code access: 4 extra cycle(s)

            // Nonsequential data fetches are also subject to a three cycle penalty
            // However the ARM9 *can* do sequential data fetches, allowing for some speedup
            // data_waitstates uses the same [region][N32, S32, N16, S16] layout as code_waitstates
            data_waitstates[0x2][0] = 1; // Main RAM N32 data access: 1 extra cycle(s)
            data_waitstates[0x3][0] = 2; // shared/ARM7 WRAM N32 data access: 2 extra cycle(s)
            data_waitstates[0x4][0] = 2; // I/O N32 data access: 2 extra cycle(s)
            data_waitstates[0x5][0] = 4; // palette N32 data access: 4 extra cycle(s)
            data_waitstates[0x6][0] = 4; // VRAM N32 data access: 4 extra cycle(s)
            data_waitstates[0x7][0] = 2; // OAM N32 data access: 2 extra cycle(s)
            data_waitstates[0xF][0] = 2; // BIOS N32 data access: 2 extra cycle(s)

            data_waitstates[0x2][1] = 1; // Main RAM S32 data access: 1 extra cycle(s)
            data_waitstates[0x3][1] = 2; // shared/ARM7 WRAM S32 data access: 2 extra cycle(s)
            data_waitstates[0x4][1] = 2; // I/O S32 data access: 2 extra cycle(s)
            data_waitstates[0x5][1] = 4; // palette S32 data access: 4 extra cycle(s)
            data_waitstates[0x6][1] = 4; // VRAM S32 data access: 4 extra cycle(s)
            data_waitstates[0x7][1] = 2; // OAM S32 data access: 2 extra cycle(s)
            data_waitstates[0xF][1] = 2; // BIOS S32 data access: 2 extra cycle(s)

            data_waitstates[0x2][2] = 1; // Main RAM N16 data access: 1 extra cycle(s)
            data_waitstates[0x3][2] = 2; // shared/ARM7 WRAM N16 data access: 2 extra cycle(s)
            data_waitstates[0x4][2] = 2; // I/O N16 data access: 2 extra cycle(s)
            data_waitstates[0x5][2] = 2; // palette N16 data access: 2 extra cycle(s)
            data_waitstates[0x6][2] = 2; // VRAM N16 data access: 2 extra cycle(s)
            data_waitstates[0x7][2] = 2; // OAM N16 data access: 2 extra cycle(s)
            data_waitstates[0xF][2] = 2; // BIOS N16 data access: 2 extra cycle(s)

            data_waitstates[0x2][3] = 1; // Main RAM S16 data access: 1 extra cycle(s)
            data_waitstates[0x3][3] = 2; // shared/ARM7 WRAM S16 data access: 2 extra cycle(s)
            data_waitstates[0x4][3] = 2; // I/O S16 data access: 2 extra cycle(s)
            data_waitstates[0x5][3] = 2; // palette S16 data access: 2 extra cycle(s)
            data_waitstates[0x6][3] = 2; // VRAM S16 data access: 2 extra cycle(s)
            data_waitstates[0x7][3] = 2; // OAM S16 data access: 2 extra cycle(s)
            data_waitstates[0xF][3] = 2; // BIOS S16 data access: 2 extra cycle(s)
        } else {
            // ARM7 - not as much here as most waitstates only equal 1
            // DS Memory Timings: https://problemkaputt.de/gbatek.htm#dsmemorytimings
            code_waitstates[0x2][0] = 2; // Main RAM N32 code access: 2 extra cycle(s)
            code_waitstates[0x5][0] = 2; // palette N32 code access: 2 extra cycle(s)
            code_waitstates[0x6][0] = 2; // VRAM N32 code access: 2 extra cycle(s)

            code_waitstates[0x2][1] = 2; // Main RAM S32 code access: 2 extra cycle(s)
            code_waitstates[0x5][1] = 2; // palette S32 code access: 2 extra cycle(s)
            code_waitstates[0x6][1] = 2; // VRAM S32 code access: 2 extra cycle(s)

            code_waitstates[0x2][2] = 1; // Main RAM N16 code access: 1 extra cycle(s)

            data_waitstates[0x2][0] = 2; // Main RAM N32 data access: 2 extra cycle(s)

            data_waitstates[0x2][1] = 2; // Main RAM S32 data access: 2 extra cycle(s)

            data_waitstates[0x2][2] = 1; // Main RAM N16 data access: 1 extra cycle(s)
        }

        Self {
            code_waitstates,
            cpu_id,
            data_waitstates,
            // ARM9 (id 0) boots with high vectors; ARM7 vectors are always at 0.
            // ARM CPU Exceptions: https://problemkaputt.de/gbatek.htm#armcpuexceptions
            exception_base: if cpu_id == 0 { 0xFFFF_0000 } else { 0 }, // ARM9 exception vectors live at FFFF0000h (CP15 high vectors), ARM7 at 00000000h
            cpu_type,
            ..Default::default()
        }
    }

    // The cross-reference has been removed, so it is no longer required.
    // Attach a CP15 coprocessor
    // pub fn set_cp15(&mut self, cp15: ()) {}

    /// Reset CPU state
    pub const fn power_on(&mut self) {
        self.halted = false;
        self.timestamp = 0;
        self.cpsr.thumb_on = false;
        self.cpsr.mode = PsrMode::Supervisor;
        self.jp(self.exception_base, true);
    }

    /// Boot directly to an entry point
    ///
    /// Register values match what the DS BIOS leaves behind before jumping to
    /// the cartridge (melonDS `NDS::SetupDirectBoot`).
    pub const fn direct_boot(&mut self, entry_point: u32) {
        self.jp(entry_point, true);
        self.regs[12] = entry_point;
        self.regs[14] = entry_point;
        let (sp, sp_irq, sp_svc) = if self.cpu_id == 0 {
            // BIOS RAM Usage: https://problemkaputt.de/gbatek.htm#biosramusage
            (0x0300_2F7C, 0x0300_3F80, 0x0300_3FC0) // ARM9 SP_usr/sys, SP_irq, SP_svc as left by the BIOS (top of DTCM/shared WRAM region)
        } else {
            (0x0380_FD80, 0x0380_FF80, 0x0380_FFC0) // ARM7 SP_usr/sys, SP_irq, SP_svc as left by the BIOS (top of ARM7 WRAM)
        };
        self.regs[13] = sp;
        self.sp_irq = sp_irq;
        self.sp_svc = sp_svc;
        self.cpsr.mode = PsrMode::System;
    }

    // pub fn run(&mut self)
    // pub fn execute(&mut self)

    /// Jump to address, optionally changing Thumb state
    pub const fn jp(&mut self, new_addr: u32, change_thumb_state: bool) {
        // #[cfg(feature = "log")]
        // log::trace!(%new_addr, %change_thumb_state);
        self.regs[15] = new_addr;

        self.add_n32_code(new_addr, 1);
        self.add_s32_code(new_addr, 1);

        if change_thumb_state {
            self.cpsr.thumb_on = (new_addr & 1) != 0;
        }

        if self.cpsr.thumb_on {
            self.regs[15] &= !1;
            self.regs[15] += 2;
        } else {
            self.regs[15] &= !3;
            self.regs[15] += 4;
        }
    }

    /// Exception handlers
    pub fn handle_undefined(&mut self) {
        let value = self.cpsr.get();
        self.spsr[PsrMode::Undefined as usize].set(value);

        self.lr_und = self.regs[15].wrapping_sub(4);
        self.update_reg_mode(PsrMode::Undefined);
        self.cpsr.mode = PsrMode::Undefined;
        self.cpsr.irq_disabled = true;
        // ARM CPU Exceptions: https://problemkaputt.de/gbatek.htm#armcpuexceptions
        self.jp(self.exception_base + 0x04, true); // vector +04h: Undefined Instruction
    }
    /// Takes the IRQ exception.
    ///
    /// ```text
    /// from: ds_emu execute() when IE & IF != 0, IME set and CPSR.I clear
    ///   1. SPSR_irq = CPSR, LR_irq = return address (+4 convention)
    ///   2. switch to IRQ mode registers, CPSR.I = 1, ARM state
    ///   3. jump to vector base + 0x18 (BIOS / high vectors on the ARM9)
    /// to:   the game's IRQ handler (via the BIOS dispatcher), which acknowledges
    ///       IF and returns with SUBS PC, LR, #4
    /// ```
    pub fn handle_irq(&mut self) {
        let value = self.cpsr.get();
        self.spsr[PsrMode::Irq as usize].set(value);

        self.lr_irq = self.regs[15] + if self.cpsr.thumb_on { 2 } else { 0 };
        self.update_reg_mode(PsrMode::Irq);
        self.cpsr.mode = PsrMode::Irq;
        self.cpsr.irq_disabled = true;
        // ARM CPU Exceptions: https://problemkaputt.de/gbatek.htm#armcpuexceptions
        self.jp(self.exception_base + 0x18, true); // vector +18h: IRQ
    }

    /// Moves the exception vectors (ARM9 CP15 control bit 13).
    pub const fn set_high_vectors(&mut self, high: bool) {
        // ARM CP15 Control Register: https://problemkaputt.de/gbatek.htm#armcp15controlregister
        self.exception_base = if high { 0xFFFF_0000 } else { 0 }; // CP15 control bit 13 (V): exception vectors at FFFF0000h vs 00000000h
    }

    /// Halt execution
    #[inline]
    pub const fn halt(&mut self) {
        self.halted = true;
    }

    /// Takes the SWI exception (real BIOS code path, `config.hle_bios` off).
    ///
    /// ```text
    /// from: SWI handlers in ds_emu arm_instruction / thumb_instruction
    ///   1. SPSR_svc = CPSR, LR_svc = address after the SWI
    ///   2. supervisor mode registers, CPSR.I = 1
    ///   3. jump to vector base + 0x08 (BIOS SWI dispatcher)
    /// ```
    pub fn handle_swi(&mut self) {
        let value = self.cpsr.get();
        self.spsr[PsrMode::Supervisor as usize].set(value);

        let mut lr = self.regs[15];
        lr -= if self.cpsr.thumb_on { 2 } else { 4 };
        self.lr_svc = lr;

        self.update_reg_mode(PsrMode::Supervisor);
        self.cpsr.mode = PsrMode::Supervisor;
        self.cpsr.irq_disabled = true;
        // ARM CPU Exceptions: https://problemkaputt.de/gbatek.htm#armcpuexceptions
        self.jp(self.exception_base + 0x08, true); // vector +08h: Software Interrupt (SWI)
    }

    /// Debug helper: prints PC, CPSR and r0-r15 to stderr.
    pub fn print_info(&self) {
        eprintln!(
            "ARM{} pc={:08X} cpsr={:08X} r={:08X?}",
            if self.cpu_id == 0 { 9 } else { 7 },
            self.get_pc(),
            self.cpsr.get(),
            self.regs
        );
    }

    /// Update register banking for a new CPU mode
    pub fn update_reg_mode(&mut self, new_mode: PsrMode) {
        if new_mode == self.cpsr.mode {
            return;
        }

        match self.cpsr.mode {
            PsrMode::Irq => {
                core::mem::swap(&mut self.regs[13], &mut self.sp_irq);
                core::mem::swap(&mut self.regs[14], &mut self.lr_irq);
            }
            PsrMode::Fiq => {
                for i in 0..5 {
                    core::mem::swap(&mut self.regs[8 + i], &mut self.fiq_regs[i]);
                }
                core::mem::swap(&mut self.regs[13], &mut self.sp_fiq);
                core::mem::swap(&mut self.regs[14], &mut self.lr_fiq);
            }
            PsrMode::Supervisor => {
                core::mem::swap(&mut self.regs[13], &mut self.sp_svc);
                core::mem::swap(&mut self.regs[14], &mut self.lr_svc);
            }
            PsrMode::Undefined => {
                core::mem::swap(&mut self.regs[13], &mut self.sp_und);
                core::mem::swap(&mut self.regs[14], &mut self.lr_und);
            }
            PsrMode::User | PsrMode::Abort | PsrMode::System => {}
        }

        match new_mode {
            PsrMode::Irq => {
                core::mem::swap(&mut self.regs[13], &mut self.sp_irq);
                core::mem::swap(&mut self.regs[14], &mut self.lr_irq);
            }
            PsrMode::Fiq => {
                for i in 0..5 {
                    core::mem::swap(&mut self.regs[8 + i], &mut self.fiq_regs[i]);
                }
                core::mem::swap(&mut self.regs[13], &mut self.sp_fiq);
                core::mem::swap(&mut self.regs[14], &mut self.lr_fiq);
            }
            PsrMode::Supervisor => {
                core::mem::swap(&mut self.regs[13], &mut self.sp_svc);
                core::mem::swap(&mut self.regs[14], &mut self.lr_svc);
            }
            PsrMode::Undefined => {
                core::mem::swap(&mut self.regs[13], &mut self.sp_und);
                core::mem::swap(&mut self.regs[14], &mut self.lr_und);
            }
            PsrMode::User | PsrMode::Abort | PsrMode::System => {}
        }
    }

    /// Debug helpers
    #[inline]
    pub const fn get_reg_name(id: u32) -> &'static str {
        match id {
            0 => "r0",
            1 => "r1",
            2 => "r2",
            3 => "r3",
            4 => "r4",
            5 => "r5",
            6 => "r6",
            7 => "r7",
            8 => "r8",
            9 => "r9",

            10 => "sl",
            11 => "fp",
            12 => "ip",
            13 => "sp",
            14 => "lr",
            15 => "pc",
            _ => "??",
        }
    }

    /// Lower-case condition suffix for the disassembler ("" for AL).
    #[inline]
    pub const fn get_condition_name(id: u32) -> &'static str {
        match id {
            0 => "eq",
            1 => "ne",
            2 => "cs",
            3 => "cc",
            4 => "mi",
            5 => "pl",
            6 => "vs",
            7 => "vc",
            8 => "hi",
            9 => "ls",
            10 => "ge",
            11 => "lt",
            12 => "gt",
            13 => "le",
            14 => "",
            _ => "??",
        }
    }

    /// CPU id: 0 = ARM9, 1 = ARM7.
    #[inline]
    pub const fn get_id(&self) -> i32 {
        self.cpu_id
    }

    /// Register access
    ///
    /// # Panics
    /// if id > 15
    pub const fn get_register(&self, id: i32) -> u32 {
        self.regs[id as usize]
    }

    /// Set value to CPU register.
    /// # Panics
    /// if id > 15
    pub const fn set_register(&mut self, id: i32, value: u32) {
        self.regs[id as usize] = value;
    }

    /// Condition code evaluation
    pub const fn check_condition(&self, condition: i32) -> bool {
        match condition {
            // ARM CPU Flags & Condition Field (cond): https://problemkaputt.de/gbatek.htm#armcpuflagsconditionfieldcond
            0x0 => self.cpsr.zero,                           // EQ - Z=1
            0x1 => !self.cpsr.zero,                          // NE - Z=0
            0x2 => self.cpsr.carry,                          // CS/HS - C=1
            0x3 => !self.cpsr.carry,                         // CC/LO - C=0
            0x4 => self.cpsr.negative,                       // MI - N=1
            0x5 => !self.cpsr.negative,                      // PL - N=0
            0x6 => self.cpsr.overflow,                       // VS - V=1
            0x7 => !self.cpsr.overflow,                      // VC - V=0
            0x8 => self.cpsr.carry && !self.cpsr.zero,       // HI - C=1 and Z=0
            0x9 => !self.cpsr.carry || self.cpsr.zero,       // LS - C=0 or Z=1
            0xA => self.cpsr.negative == self.cpsr.overflow, // GE - N=V
            0xB => self.cpsr.negative != self.cpsr.overflow, // LT - N<>V
            0xC => !self.cpsr.zero && (self.cpsr.negative == self.cpsr.overflow), // GT - Z=0 and N=V
            0xD => self.cpsr.zero || (self.cpsr.negative != self.cpsr.overflow), // LE - Z=1 or N<>V
            0xE => true,                                                         // AL - always
            _ => false,
        }
    }

    /// Upper-case condition suffix (`None` for AL / NV).
    pub const fn print_condition(condition: u32) -> Option<&'static str> {
        Some(match condition {
            0x0 => "EQ", // EQ
            0x1 => "NE", // NE
            0x2 => "CS", // CS
            0x3 => "CC", // CC
            0x4 => "MI", // MI
            0x5 => "PL", // PL
            0x6 => "VS", // VS
            0x7 => "VC", // VC
            0x8 => "HI", // HI
            0x9 => "LS", // LS
            0xA => "GE", // GE
            0xB => "LT", // LT
            0xC => "GT", // GT
            0xD => "LE", // LE
            _ => return None,
        })
    }

    /// Cycles executed so far (ARM9: 66 MHz units, ARM7: 33 MHz).
    pub const fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    /// Cycles taken by the last execute() (timestamp - last_timestamp).
    pub const fn cycles_ran(&self) -> i64 {
        self.timestamp.wrapping_sub(self.last_timestamp) as i64
    }

    /// Get program counter (Current instruction pointer)
    ///
    /// Via register.15th
    pub const fn get_pc(&self) -> u32 {
        self.regs[REG_PC as usize]
    }

    /// The instruction fetched by execute() and being interpreted.
    pub const fn get_current_instr(&self) -> u32 {
        self.current_instr
    }

    /// CPSR access
    #[inline]
    pub const fn get_cpsr(&self) -> &PsrFlags {
        &self.cpsr
    }

    /// CPSR access
    #[inline]
    pub const fn get_cpsr_mut(&mut self) -> &mut PsrFlags {
        &mut self.cpsr
    }

    //WaitState bullshit
    /// Adds the time of `cycles` non-sequential 32-bit code fetches at `address`.
    pub const fn add_n32_code(&mut self, address: u32, cycles: i32) {
        let idx = ((address & 0x0F00_0000) >> 24) as usize; // memory region = address bits 24-27
        self.timestamp += (1 + self.code_waitstates[idx][0]) as u64 * cycles as u64;
    }
    /// Adds the time of `cycles` sequential 32-bit code fetches at `address`.
    pub const fn add_s32_code(&mut self, address: u32, cycles: i32) {
        let idx = ((address & 0x0F00_0000) >> 24) as usize; // memory region = address bits 24-27
        self.timestamp += (1 + self.code_waitstates[idx][1]) as u64 * cycles as u64;
    }
    /// Adds the time of `cycles` non-sequential 16-bit code fetches at `address`.
    pub const fn add_n16_code(&mut self, address: u32, cycles: i32) {
        let idx = ((address & 0x0F00_0000) >> 24) as usize; // memory region = address bits 24-27
        self.timestamp += (1 + self.code_waitstates[idx][2]) as u64 * cycles as u64;
    }
    /// Adds the time of `cycles` sequential 16-bit code fetches at `address`.
    pub const fn add_s16_code(&mut self, address: u32, cycles: i32) {
        let idx = ((address & 0x0F00_0000) >> 24) as usize; // memory region = address bits 24-27
        self.timestamp += (1 + self.code_waitstates[idx][3]) as u64 * cycles as u64;
    }
    /// Adds the time of `cycles` non-sequential 32-bit data accesses at `address`.
    pub const fn add_n32_data(&mut self, address: u32, cycles: i32) {
        let index = ((address & 0x0F000000) >> 24) as usize; // memory region = address bits 24-27
        let data_waitstates = self.data_waitstates[index][0] as u64;
        self.timestamp += (1 + data_waitstates) * cycles as u64;
    }
    /// Adds the time of `cycles` sequential 32-bit data accesses at `address`.
    pub const fn add_s32_data(&mut self, address: u32, cycles: i32) {
        let index = ((address & 0x0F000000) >> 24) as usize; // memory region = address bits 24-27
        let data_waitstates = self.data_waitstates[index][1] as u64;
        self.timestamp += (1 + data_waitstates) * cycles as u64;
    }
    /// Adds the time of `cycles` non-sequential 16-bit data accesses at `address`.
    pub const fn add_n16_data(&mut self, address: u32, cycles: i32) {
        let index = ((address & 0x0F000000) >> 24) as usize; // memory region = address bits 24-27
        let data_waitstates = self.data_waitstates[index][2] as u64;
        self.timestamp += (1 + data_waitstates) * cycles as u64;
    }
    /// Adds the time of `cycles` sequential 16-bit data accesses at `address`.
    pub const fn add_s16_data(&mut self, address: u32, cycles: i32) {
        let index = ((address & 0x0F000000) >> 24) as usize; // memory region = address bits 24-27
        let data_waitstates = self.data_waitstates[index][3] as u64;
        self.timestamp += (1 + data_waitstates) * cycles as u64;
    }

    /// Cycle accounting
    pub const fn add_internal_cycles(&mut self, cycles: i32) {
        self.timestamp += cycles as u64;
    }

    /// Adds coprocessor (CP15) access cycles.
    pub const fn add_cop_cycles(&mut self, cycles: i32) {
        self.timestamp += cycles as u64;
    }

    // All data manipulation methods here
    /// Writes an ALU result to `dst` and reports whether N/Z/C/V should
    /// then be updated.
    ///
    /// For `dst == PC` the S bit means "exception return": CPSR is restored
    /// from SPSR (register bank and Thumb state included) and the flags come
    /// from the SPSR rather than from the result. Without S it is a plain
    /// branch; ALU writes to PC never interwork, so the current state is
    /// kept.
    fn alu_writeback(&mut self, dst: u32, result: u32, set_flags: bool) -> bool {
        if dst == REG_PC {
            if set_flags {
                self.spsr_to_cpsr();
            }
            self.jp(result, false);
            false
        } else {
            self.set_register(dst as i32, result);
            set_flags
        }
    }

    /// AND: `Rd = Rn & op` (flags N/Z when S).
    pub fn andd(&mut self, dst: i32, src: i32, operand: i32, set_condition_codes: bool) {
        let result = (src & operand) as u32;
        if self.alu_writeback(dst as u32, result, set_condition_codes) {
            self.set_zero_neg_flags(result);
        }
    }

    /// ORR: `Rd = Rn | op`.
    pub fn orr(&mut self, dst: i32, src: i32, operand: i32, set_condition_codes: bool) {
        let result = (src | operand) as u32;
        if self.alu_writeback(dst as u32, result, set_condition_codes) {
            self.set_zero_neg_flags(result);
        }
    }

    /// XOR
    pub fn eor(&mut self, dst: i32, src: i32, operand: i32, set_condition_codes: bool) {
        let result = (src ^ operand) as u32;
        if self.alu_writeback(dst as u32, result, set_condition_codes) {
            self.set_zero_neg_flags(result);
        }
    }

    /// ADD: `Rd = Rn + op` (N/Z/C/V when S).
    pub fn add(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let result = src.wrapping_add(operand);
        if self.alu_writeback(dst, result, set_condition_codes) {
            self.set_zero_neg_flags(result);
            self.set_cv_add_flags(src, operand, result);
        }
    }

    /// SUB: `Rd = Rn - op` (N/Z/C/V when S).
    pub fn sub(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let result = src.wrapping_sub(operand);
        if self.alu_writeback(dst, result, set_condition_codes) {
            self.set_zero_neg_flags(result);
            self.set_cv_sub_flags(src, operand, result);
        }
    }

    /// Add with carry: `src + operand + C`.
    pub fn adc(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let wide = src as u64 + operand as u64 + self.cpsr.carry as u64;
        let result = wide as u32;
        if self.alu_writeback(dst, result, set_condition_codes) {
            self.set_zero_neg_flags(result);
            self.cpsr.carry = wide > 0xFFFF_FFFF; // carry out of bit 31
            self.cpsr.overflow = add_overflow(src, operand, result);
        }
    }

    /// Subtract with carry: `src - operand - !C`.
    pub fn sbc(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let borrow = !self.cpsr.carry as u32;
        let result = src.wrapping_sub(operand).wrapping_sub(borrow);
        if self.alu_writeback(dst, result, set_condition_codes) {
            self.set_zero_neg_flags(result);
            self.cpsr.carry = src as u64 >= operand as u64 + borrow as u64;
            self.cpsr.overflow = sub_overflow(src, operand, result);
        }
    }

    /// CMP: flags of `x - y`.
    pub const fn cmp(&mut self, x: u32, y: u32) {
        let result = x.wrapping_sub(y);
        self.set_zero_neg_flags(result);
        self.set_cv_sub_flags(x, y, result);
    }
    /// CMN: flags of `x + y`.
    pub const fn cmn(&mut self, x: u32, y: u32) {
        let result = x.wrapping_add(y);
        self.set_zero_neg_flags(result);
        self.set_cv_add_flags(x, y, result);
    }
    /// TST: N/Z of `x & y`.
    pub const fn tst(&mut self, x: u32, y: u32) {
        self.set_zero_neg_flags(x & y);
    }
    /// TEQ: N/Z of `x ^ y`.
    pub const fn teq(&mut self, x: u32, y: u32) {
        self.set_zero_neg_flags(x ^ y);
    }

    /// Executes `MOV Rd, operand` (`MOVS PC, LR` is an exception return).
    pub fn mov(&mut self, dst: u32, operand: u32, alter_flags: bool) {
        if self.alu_writeback(dst, operand, alter_flags) {
            self.set_zero_neg_flags(operand);
        }
    }

    /// MUL: `Rd = Rm * Rs` (N/Z when S).
    pub const fn mul(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let result = src.wrapping_mul(operand);
        self.set_register(dst as i32, result);

        if set_condition_codes {
            self.set_zero_neg_flags(result);
        }
    }

    /// BIC: `Rd = Rn & !op`.
    pub fn bic(&mut self, dst: u32, src: u32, operand: u32, alter_flags: bool) {
        let result = src & !operand;
        if self.alu_writeback(dst, result, alter_flags) {
            self.set_zero_neg_flags(result);
        }
    }

    /// MVN: `Rd = !op`.
    pub fn mvn(&mut self, dst: u32, operand: u32, alter_flags: bool) {
        if self.alu_writeback(dst, !operand, alter_flags) {
            self.set_zero_neg_flags(!operand);
        }
    }

    /// MRS: copies CPSR or the current SPSR to a register.
    pub const fn mrs(&mut self, instruction: u32) {
        let using_cpsr = (instruction & (1 << 22)) == 0;
        // ARM Opcodes: PSR Transfer (MRS, MSR): https://problemkaputt.de/gbatek.htm#armopcodespsrtransfermrsmsr
        let dst = (instruction >> 12) & 0xF; // Rd (bits 12-15)

        if using_cpsr {
            self.set_register(dst as i32, self.cpsr.get());
        } else {
            self.set_register(dst as i32, self.spsr[self.cpsr.mode as usize].get());
        }
    }

    /// MSR: writes CPSR/SPSR fields (mode change re-banks registers).
    pub fn msr(&mut self, instruction: u32) {
        // ARM Opcodes: PSR Transfer (MRS, MSR): https://problemkaputt.de/gbatek.htm#armopcodespsrtransfermrsmsr
        let is_imm = (instruction & (1 << 25)) != 0; // I bit (25): immediate operand
        let using_cpsr = (instruction & (1 << 22)) == 0; // Psr bit (22): 0 = CPSR, 1 = SPSR
        let mode = self.cpsr.mode;

        let mut value = if using_cpsr { self.cpsr.get() } else { self.spsr[mode as usize].get() };

        let source = if is_imm {
            // ARM Opcodes: PSR Transfer (MRS, MSR): https://problemkaputt.de/gbatek.htm#armopcodespsrtransfermrsmsr
            let s = instruction & 0xFF; // 8-bit immediate (bits 0-7)
            let shift = (instruction & 0xF00) >> 7; // rotate field (bits 8-11) * 2
            self.rotr32(s, shift, false)
        } else {
            self.get_register((instruction & 0xF) as i32) // Rm (bits 0-3)
        };

        let mut bitmask: u32 = 0;

        if (instruction & (1 << 19)) != 0 {
            // field mask bit 19 (f): flags
            bitmask |= 0xFF000000; // field mask bit 19 (f): flags byte, bits 24-31
        }

        if (instruction & (1 << 16)) != 0 {
            // field mask bit 16 (c): control
            bitmask |= 0xFF; // field mask bit 16 (c): control byte, bits 0-7
        }

        if self.cpsr.mode == PsrMode::User {
            bitmask &= 0xFFFFFF00; // User mode may not change the control byte
        }

        if using_cpsr {
            bitmask &= 0xFFFFFFDF; // T bit (bit 5) cannot be changed by MSR
        }

        value &= !bitmask;
        value |= source & bitmask;

        if using_cpsr {
            if let Some(mode) = PsrMode::from_u32(value & 0x1F) {
                // CPSR mode bits M4-M0 (bits 0-4)
                self.update_reg_mode(mode);
            } else {
                #[cfg(feature = "log")]
                log::warn!("Invalid PsrMode: new_cpsr = {using_cpsr}");
            }
        }

        let psr: &mut PsrFlags =
            if using_cpsr { &mut self.cpsr } else { &mut self.spsr[mode as usize] };
        psr.set(value);
    }

    /// Sets or clears CPSR.Z.
    pub const fn set_zero(&mut self, cond: bool) {
        self.cpsr.zero = cond;
    }

    /// Sets or clears CPSR.N.
    pub const fn set_neg(&mut self, cond: bool) {
        self.cpsr.negative = cond;
    }

    /// Flag helpers
    pub const fn set_zero_neg_flags(&mut self, value: u32) {
        self.cpsr.zero = value == 0;
        self.cpsr.negative = (value & (1 << 31)) != 0; // N flag = result bit 31
    }

    /// C and V of an addition.
    pub const fn set_cv_add_flags(&mut self, a: u32, b: u32, result: u32) {
        self.cpsr.carry = carry_add(a, b);
        self.cpsr.overflow = add_overflow(a, b, result);
    }

    /// C and V of a subtraction (C = no borrow).
    pub const fn set_cv_sub_flags(&mut self, a: u32, b: u32, result: u32) {
        self.cpsr.carry = a >= b;
        self.cpsr.overflow = sub_overflow(a, b, result);
    }

    /// Exception return: CPSR = SPSR of the current mode (re-banks registers).
    pub fn spsr_to_cpsr(&mut self) {
        let new_cpsr = self.spsr[self.cpsr.mode as usize].get();

        if let Some(mode) = PsrMode::from_u32(new_cpsr & 0x1F) {
            // CPSR mode bits M4-M0 (bits 0-4)
            self.update_reg_mode(mode);
            self.cpsr.set(new_cpsr);
        } else {
            #[cfg(feature = "log")]
            log::warn!("Invalid PsrMode: new_cpsr = {new_cpsr}");
        }
    }

    /// Applies barrel-shifter flags: N/Z from `result`, C from `carry` when
    /// the shift produced a carry-out (`None` leaves C unchanged).
    const fn shift_flags(&mut self, result: u32, carry: Option<bool>, alter_flags: bool) {
        if alter_flags {
            self.set_zero_neg_flags(result);
            if let Some(carry) = carry {
                self.cpsr.carry = carry;
            }
        }
    }

    /// Logical shift left. `shift` is the effective amount (0 = no shift,
    /// carry unchanged; 32 = result 0, carry = bit 0; >32 = 0, carry 0).
    pub const fn lsl(&mut self, value: u32, shift: i32, alter_flags: bool) -> u32 {
        let (result, carry) = match shift as u32 {
            0 => (value, None),
            s @ 1..=31 => (value << s, Some((value >> (32 - s)) & 1 != 0)),
            32 => (0, Some(value & 1 != 0)),
            _ => (0, Some(false)),
        };
        self.shift_flags(result, carry, alter_flags);
        result
    }

    /// Logical shift left by 32 bits.
    pub const fn lsl_32(&mut self, value: u32, alter_flags: bool) -> u32 {
        self.lsl(value, 32, alter_flags)
    }

    /// Logical shift right. Callers translate the immediate encoding
    /// `LSR #0` (meaning 32) themselves; `shift == 0` here means no shift.
    pub const fn lsr(&mut self, value: u32, shift: i32, alter_flags: bool) -> u32 {
        let (result, carry) = match shift as u32 {
            0 => (value, None),
            s @ 1..=31 => (value >> s, Some((value >> (s - 1)) & 1 != 0)),
            32 => (0, Some(value >> 31 != 0)),
            _ => (0, Some(false)),
        };
        self.shift_flags(result, carry, alter_flags);
        result
    }

    /// Logical shift right by 32 (`LSR #0` encoding).
    pub const fn lsr_32(&mut self, value: u32, alter_flags: bool) -> u32 {
        self.lsr(value, 32, alter_flags)
    }

    /// Arithmetic (sign-filling) shift right; amounts >= 32 fill with bit 31.
    pub const fn asr(&mut self, value: u32, shift: i32, alter_flags: bool) -> u32 {
        let (result, carry) = match shift as u32 {
            0 => (value, None),
            s @ 1..=31 => (((value as i32) >> s) as u32, Some((value >> (s - 1)) & 1 != 0)),
            _ => (((value as i32) >> 31) as u32, Some(value >> 31 != 0)),
        };
        self.shift_flags(result, carry, alter_flags);
        result
    }

    /// Arithmetic shift right by 32 (`ASR #0` encoding).
    pub const fn asr_32(&mut self, value: u32, alter_flags: bool) -> u32 {
        self.asr(value, 32, alter_flags)
    }

    /// Rotate right by one through carry (`ROR #0` encoding).
    pub const fn rrx(&mut self, value: u32, alter_flags: bool) -> u32 {
        let mut result = value;
        result >>= 1;
        if self.cpsr.carry {
            result |= 1 << 31; // RRX: old carry into bit 31
        } else {
            result |= 0;
        };

        if alter_flags {
            self.set_zero_neg_flags(result);
            self.cpsr.carry = (value & 0x1) != 0; // RRX: carry out is bit 0
        }
        result
    }

    /// Rotate right. `c == 0` leaves the value and carry unchanged; any other
    /// amount rotates by `c % 32` with carry = bit 31 of the result (so a
    /// multiple of 32 yields the value unchanged and carry = bit 31).
    pub const fn rotr32(&mut self, n: u32, c: u32, alter_flags: bool) -> u32 {
        if c == 0 {
            self.shift_flags(n, None, alter_flags);
            return n;
        }
        let result = n.rotate_right(c & 0x1F); // rotate amount modulo 32
        self.shift_flags(result, Some(result >> 31 != 0), alter_flags);
        result
    }
}
