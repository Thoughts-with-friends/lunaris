//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! cpuinsters.hpp
//!

/// General-purpose register indices
pub const REG_SP: u32 = 13;
pub const REG_LR: u32 = 14;
pub const REG_PC: u32 = 15;

#[inline]
const fn carry_add(a: u32, b: u32) -> bool {
    (0xFFFF_FFFF_u32.wrapping_sub(a)) < b
}

#[inline]
const fn carry_sub(a: u32, b: u32) -> bool {
    a >= b
}

#[inline]
const fn add_overflow(a: u32, b: u32, result: u32) -> bool {
    (((a ^ b) & 0x8000_0000) == 0) && (((a ^ result) & 0x8000_0000) != 0)
}

#[inline]
const fn sub_overflow(a: u32, b: u32, result: u32) -> bool {
    (((a ^ b) & 0x8000_0000) != 0) && (((a ^ result) & 0x8000_0000) != 0)
}

/// ARM processor modes (CPSR.M)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum PsrMode {
    /// User mode
    #[default]
    User = 0x10,
    /// FIQ (Fast Interrupt) mode
    Fiq = 0x11,
    /// IRQ (Interrupt) mode
    Irq = 0x12,
    /// Supervisor mode
    Supervisor = 0x13,
    /// Abort mode
    Abort = 0x17,
    /// Undefined instruction mode
    Undefined = 0x1B,
    /// System mode
    System = 0x1F,
}

impl PsrMode {
    /// Const-safe conversion from raw CPSR.M value
    pub const fn from_u32(value: u32) -> Option<Self> {
        Some(match value {
            0x10 => Self::User,
            0x11 => Self::Fiq,
            0x12 => Self::Irq,
            0x13 => Self::Supervisor,
            0x17 => Self::Abort,
            0x1B => Self::Undefined,
            0x1F => Self::System,
            _ => return None,
        })
    }
}

/// Program Status Register flags
#[derive(Copy, Clone, Debug, Default)]
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
        reg |= (self.negative as u32) << 31;
        reg |= (self.zero as u32) << 30;
        reg |= (self.carry as u32) << 29;
        reg |= (self.overflow as u32) << 28;
        reg |= (self.sticky_overflow as u32) << 27;

        reg |= (self.irq_disabled as u32) << 7;
        reg |= (self.fiq_disabled as u32) << 6;
        reg |= (self.thumb_on as u32) << 5;

        reg |= self.mode as u32;

        reg
    }

    /// Load flags from a 32-bit CPSR/SPSR value
    pub const fn set(&mut self, value: u32) {
        self.negative = (value & (1 << 31)) != 0;
        self.zero = (value & (1 << 30)) != 0;
        self.carry = (value & (1 << 29)) != 0;
        self.overflow = (value & (1 << 28)) != 0;
        self.sticky_overflow = (value & (1 << 27)) != 0;

        self.irq_disabled = (value & (1 << 7)) != 0;
        self.fiq_disabled = (value & (1 << 6)) != 0;
        self.thumb_on = (value & (1 << 5)) != 0;

        if let Some(mode) = PsrMode::from_u32(value & 0x1F) {
            self.mode = mode;
        } else {
            #[cfg(feature = "tracing")]
            tracing::warn!("invalid PsrMode: {value}");
        }
    }
}

/// ARM CPU core
#[derive(Debug, Default)]
pub struct ArmCpu {
    /// CPU ID (ARM9 / ARM7 etc.)
    cpu_id: i32,

    /// Halt state
    halted: bool,

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
    regs: [u32; 16],

    /// Current Program Status Register
    cpsr: PsrFlags,

    /// Saved Program Status Registers (indexed by mode)
    spsr: [PsrFlags; 0x20],

    /// Exception vector base address
    exception_base: u32,

    /// Cycle timestamp
    timestamp: u64,
    last_timestamp: u64,

    /// Currently executing instruction
    current_instr: u32,

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

impl ArmCpu {
    /// Create a new ARM CPU
    pub fn new(cpu_id: i32) -> Self {
        let mut code_waitstates: [[i32; 4]; 16] = [[0; 4]; 16];
        let mut data_waitstates: [[i32; 4]; 16] = [[0; 4]; 16];

        //Fill waitstates with dummy values to prevent bugs
        for i in 0..4 {
            for j in 0..5 {
                code_waitstates[i][j] = 1;
            }
            for j in 0..6 {
                code_waitstates[i][j] = 1;
            }
        }

        //Initialize waitstates based upon GBATek's info
        //Note that 8-bit reads/writes have the same timings as 16 bit
        if cpu_id <= 0 {
            //ARM9 - timings here are doubled to represent 66 MHz
            //Of interest is that the ARM9 is incapable of sequential code fetches
            //Thus ALL code fetches are subject to the three cycle penalty... so much for 66 MHz
            code_waitstates[0x2][0] = 1;
            code_waitstates[0x3][0] = 8;
            code_waitstates[0x4][0] = 8;
            code_waitstates[0x5][0] = 10;
            code_waitstates[0x6][0] = 10;
            code_waitstates[0x7][0] = 8;
            code_waitstates[0xF][0] = 8;

            code_waitstates[0x2][1] = 1;
            code_waitstates[0x3][1] = 8;
            code_waitstates[0x4][1] = 8;
            code_waitstates[0x5][1] = 10;
            code_waitstates[0x6][1] = 10;
            code_waitstates[0x7][1] = 8;
            code_waitstates[0xF][1] = 8;

            code_waitstates[0x2][2] = 1;
            code_waitstates[0x3][2] = 4;
            code_waitstates[0x4][2] = 4;
            code_waitstates[0x5][2] = 5;
            code_waitstates[0x6][2] = 5;
            code_waitstates[0x7][2] = 4;
            code_waitstates[0xF][2] = 4;

            code_waitstates[0x2][3] = 1;
            code_waitstates[0x3][3] = 4;
            code_waitstates[0x4][3] = 4;
            code_waitstates[0x5][3] = 5;
            code_waitstates[0x6][3] = 5;
            code_waitstates[0x7][3] = 4;
            code_waitstates[0xF][3] = 4;

            //Nonsequential data fetches are also subject to a three cycle penalty
            //However the ARM9 *can* do sequential data fetches, allowing for some speedup
            data_waitstates[0x2][0] = 1;
            data_waitstates[0x3][0] = 2;
            data_waitstates[0x4][0] = 2;
            data_waitstates[0x5][0] = 4;
            data_waitstates[0x6][0] = 4;
            data_waitstates[0x7][0] = 2;
            data_waitstates[0xF][0] = 2;

            data_waitstates[0x2][1] = 1;
            data_waitstates[0x3][1] = 2;
            data_waitstates[0x4][1] = 2;
            data_waitstates[0x5][1] = 4;
            data_waitstates[0x6][1] = 4;
            data_waitstates[0x7][1] = 2;
            data_waitstates[0xF][1] = 2;

            data_waitstates[0x2][2] = 1;
            data_waitstates[0x3][2] = 2;
            data_waitstates[0x4][2] = 2;
            data_waitstates[0x5][2] = 2;
            data_waitstates[0x6][2] = 2;
            data_waitstates[0x7][2] = 2;
            data_waitstates[0xF][2] = 2;

            data_waitstates[0x2][3] = 1;
            data_waitstates[0x3][3] = 2;
            data_waitstates[0x4][3] = 2;
            data_waitstates[0x5][3] = 2;
            data_waitstates[0x6][3] = 2;
            data_waitstates[0x7][3] = 2;
            data_waitstates[0xF][3] = 2;
        } else {
            //ARM7 - not as much here as most waitstates only equal 1
            code_waitstates[0x2][0] = 2;
            code_waitstates[0x5][0] = 2;
            code_waitstates[0x6][0] = 2;

            code_waitstates[0x2][1] = 2;
            code_waitstates[0x5][1] = 2;
            code_waitstates[0x6][1] = 2;

            code_waitstates[0x2][2] = 1;

            data_waitstates[0x2][0] = 2;

            data_waitstates[0x2][1] = 2;

            data_waitstates[0x2][2] = 1;
        }

        Self {
            code_waitstates,
            cpu_id,
            data_waitstates,
            exception_base: if cpu_id <= 0 { 0 } else { 0xFFFF0000 },
            ..Default::default()
        }
    }

    /// Attach a CP15 coprocessor
    pub fn set_cp15(&mut self, cp15: ()) {
        todo!()
    }

    /// Reset CPU state
    pub const fn power_on(&mut self) {
        self.halted = false;
        self.timestamp = 0;
        self.cpsr.thumb_on = false;
        self.cpsr.mode = PsrMode::Supervisor;
        self.jp(self.exception_base, true);
    }

    /// Boot directly to an entry point
    pub const fn direct_boot(&mut self, entry_point: u32) {
        self.jp(entry_point, true);
        self.regs[12] = entry_point;
        self.regs[13] = entry_point;
        self.cpsr.mode = PsrMode::System;
    }

    /// Run continuously
    pub fn run(&mut self) {
        todo!()
    }

    /// Execute a single instruction
    pub fn execute(&mut self) {
        todo!()
    }

    /// Jump to address, optionally changing Thumb state
    pub const fn jp(&mut self, new_addr: u32, change_thumb_state: bool) {
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
        self.jp(self.exception_base + 0x04, true);
    }
    pub fn handle_irq(&mut self) {
        let value = self.cpsr.get();
        self.spsr[PsrMode::Irq as usize].set(value);

        self.lr_irq = self.regs[15] + if self.cpsr.thumb_on { 2 } else { 0 };
        self.update_reg_mode(PsrMode::Irq);
        self.cpsr.mode = PsrMode::Irq;
        self.cpsr.irq_disabled = true;
        self.jp(self.exception_base + 0x18, true);
    }

    /// Halt execution
    #[inline]
    pub const fn halt(&mut self) {
        self.halted = true;
    }

    pub fn handle_swi(&mut self) {
        let value = self.cpsr.get();
        self.spsr[PsrMode::Supervisor as usize].set(value);

        let mut lr = self.regs[15];
        lr -= if self.cpsr.thumb_on { 2 } else { 4 };
        self.lr_svc = lr;

        self.update_reg_mode(PsrMode::Supervisor);
        self.cpsr.mode = PsrMode::Supervisor;
        self.cpsr.irq_disabled = true;
        self.jp(self.exception_base + 0x08, true);
    }

    /// Debug helpers
    pub fn print_info(&self) {
        todo!()
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
    pub const fn get_reg_name(id: i32) -> &'static str {
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

    #[inline]
    pub const fn get_condition_name(id: i32) -> &'static str {
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
            0x0 => self.cpsr.zero,
            0x1 => !self.cpsr.zero,
            0x2 => self.cpsr.carry,
            0x3 => !self.cpsr.carry,
            0x4 => self.cpsr.negative,
            0x5 => !self.cpsr.negative,
            0x6 => self.cpsr.overflow,
            0x7 => !self.cpsr.overflow,
            0x8 => self.cpsr.carry && !self.cpsr.zero,
            0x9 => !self.cpsr.carry || self.cpsr.zero,
            0xA => self.cpsr.negative == self.cpsr.overflow,
            0xB => self.cpsr.negative != self.cpsr.overflow,
            0xC => !self.cpsr.zero && (self.cpsr.negative == self.cpsr.overflow),
            0xD => self.cpsr.zero || (self.cpsr.negative != self.cpsr.overflow),
            0xE => true,
            _ => false,
        }
    }

    pub fn print_condition(&self, condition: i32) {
        todo!()
    }

    pub const fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    pub const fn cycles_ran(&self) -> i64 {
        (self.timestamp - self.last_timestamp) as i64
    }

    pub const fn get_pc(&self) -> u32 {
        self.regs[REG_PC as usize]
    }

    pub const fn get_current_instr(&self) -> u32 {
        self.current_instr
    }

    /// CPSR access
    pub const fn get_cpsr(&self) -> &PsrFlags {
        &self.cpsr
    }

    /// Memory access
    pub fn read_word(&mut self, address: u32) -> u32 {
        todo!()
    }
    pub fn read_halfword(&mut self, address: u32) -> u16 {
        todo!()
    }
    pub fn read_byte(&mut self, address: u32) -> u8 {
        todo!()
    }

    pub fn write_word(&mut self, address: u32, value: u32) {
        todo!()
    }
    pub fn write_halfword(&mut self, address: u32, value: u16) {
        todo!()
    }
    pub fn write_byte(&mut self, address: u32, value: u8) {
        todo!()
    }

    //WaitState bullshit
    pub const fn add_n32_code(&mut self, address: u32, cycles: i32) {
        let idx = ((address & 0x0F00_0000) >> 24) as usize;
        self.timestamp += (1 + self.code_waitstates[idx][0]) as u64 * cycles as u64;
    }
    pub const fn add_s32_code(&mut self, address: u32, cycles: i32) {
        let idx = ((address & 0x0F00_0000) >> 24) as usize;
        self.timestamp += (1 + self.code_waitstates[idx][1]) as u64 * cycles as u64;
    }
    pub const fn add_n16_code(&mut self, address: u32, cycles: i32) {
        let idx = ((address & 0x0F00_0000) >> 24) as usize;
        self.timestamp += (1 + self.code_waitstates[idx][2]) as u64 * cycles as u64;
    }
    pub const fn add_s16_code(&mut self, address: u32, cycles: i32) {
        let idx = ((address & 0x0F00_0000) >> 24) as usize;
        self.timestamp += (1 + self.code_waitstates[idx][3]) as u64 * cycles as u64;
    }
    pub const fn add_n32_data(&mut self, address: u32, cycles: i32) {
        let index = ((address & 0x0F000000) >> 24) as usize;
        let data_waitstates = self.data_waitstates[index][0] as u64;
        self.timestamp += (1 + data_waitstates) * cycles as u64;
    }
    pub const fn add_s32_data(&mut self, address: u32, cycles: i32) {
        let index = ((address & 0x0F000000) >> 24) as usize;
        let data_waitstates = self.data_waitstates[index][1] as u64;
        self.timestamp += (1 + data_waitstates) * cycles as u64;
    }
    pub const fn add_n16_data(&mut self, address: u32, cycles: i32) {
        let index = ((address & 0x0F000000) >> 24) as usize;
        let data_waitstates = self.data_waitstates[index][2] as u64;
        self.timestamp += (1 + data_waitstates) * cycles as u64;
    }
    pub const fn add_s16_data(&mut self, address: u32, cycles: i32) {
        let index = ((address & 0x0F000000) >> 24) as usize;
        let data_waitstates = self.data_waitstates[index][3] as u64;
        self.timestamp += (1 + data_waitstates) * cycles as u64;
    }

    /// Cycle accounting
    pub const fn add_internal_cycles(&mut self, cycles: i32) {
        self.timestamp += cycles as u64;
    }

    pub const fn add_cop_cycles(&mut self, cycles: i32) {
        self.timestamp += cycles as u64;
    }

    // All data manipulation methods here
    pub const fn andd(&mut self, dst: i32, src: i32, operand: i32, set_condition_codes: bool) {
        let result = (src & operand) as u32;
        self.set_register(dst, result);

        if set_condition_codes {
            self.set_zero_neg_flags(result);
        }
    }
    pub const fn orr(&mut self, dst: i32, src: i32, operand: i32, set_condition_codes: bool) {
        let result = (src | operand) as u32;
        self.set_register(dst, result);

        if set_condition_codes {
            self.set_zero_neg_flags(result);
        }
    }
    /// XOR
    pub const fn eor(&mut self, dst: i32, src: i32, operand: i32, set_condition_codes: bool) {
        let result = (src ^ operand) as u32;
        self.set_register(dst, result);

        if set_condition_codes {
            self.set_zero_neg_flags(result);
        }
    }
    pub const fn add(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let unsigned_result: u64 = (src + operand) as u64;

        if dst == REG_PC {
            if set_condition_codes {
                #[cfg(feature = "tracing")]
                tracing::error!("PC: dst and set_condition_codes is unsupported.");
            } else {
                self.jp((unsigned_result & 0xFFFFFFFF) as u32, true);
            }
        } else {
            self.set_register(dst as i32, (unsigned_result & 0xFFFFFFFF) as u32);
            if set_condition_codes {
                self.cmp(src, operand);
            }
        }
    }
    pub fn sub(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let unsigned_result: u64 = (src - operand) as u64;

        if dst == REG_PC {
            if set_condition_codes {
                let index = self.cpsr.mode as usize;
                self.update_reg_mode(self.spsr[index].mode);
                self.cpsr.set(self.spsr[index].get());
                self.jp((unsigned_result & 0xFFFFFFFF) as u32, false);
            } else {
                self.jp((unsigned_result & 0xFFFFFFFF) as u32, true);
            }
        } else {
            self.set_register(dst as i32, (unsigned_result & 0xFFFFFFFF) as u32);
            if set_condition_codes {
                self.cmp(src, operand);
            }
        }
    }
    pub const fn adc(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let carry = if self.cpsr.carry { 1 } else { 0 };
        self.add(dst, src + carry, operand, set_condition_codes);

        if set_condition_codes {
            let temp = src + operand;
            let res = temp + carry;
            self.cpsr.carry = carry_add(src, operand) | carry_add(temp, carry);
            self.cpsr.overflow = add_overflow(src, operand, temp) | add_overflow(temp, carry, res);
        }
    }
    pub const fn sbc(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let borrow = if self.cpsr.carry { 0 } else { 1 };
        self.add(dst, src + borrow, operand, set_condition_codes);

        if set_condition_codes {
            let temp = src + operand;
            let res = temp - borrow;
            self.cpsr.carry = carry_sub(src, operand) | carry_sub(temp, borrow);
            self.cpsr.overflow = sub_overflow(src, operand, temp) | sub_overflow(temp, borrow, res);
        }
    }

    pub const fn cmp(&mut self, x: u32, y: u32) {
        let result = x - y;
        self.set_zero_neg_flags(result);
        self.set_cv_sub_flags(x, y, result);
    }
    pub const fn cmn(&mut self, x: u32, y: u32) {
        let result = x + y;
        self.set_zero_neg_flags(result);
        self.set_cv_add_flags(x, y, result);
    }
    pub const fn tst(&mut self, x: u32, y: u32) {
        self.set_zero_neg_flags(x & y);
    }
    pub const fn teq(&mut self, x: u32, y: u32) {
        self.set_zero_neg_flags(x ^ y);
    }

    pub fn mov(&mut self, dst: u32, operand: u32, alter_flags: bool) {
        if dst == REG_PC && alter_flags {
            let index = self.cpsr.mode;
            self.update_reg_mode(self.spsr[index as usize].mode);
            self.cpsr.set(self.spsr[index as usize].get());
            self.jp(operand, true);
        }
    }

    pub const fn mul(&mut self, dst: u32, src: u32, operand: u32, set_condition_codes: bool) {
        let result = (src * operand) as u64;
        let truncated = (result & 0xFFFFFFFF) as u32;
        self.set_register(dst as i32, truncated);

        if set_condition_codes {
            self.set_zero_neg_flags(truncated);
        }
    }

    pub const fn bic(&mut self, dst: u32, src: u32, operand: u32, alter_flags: bool) {
        let result = src & !operand;
        self.set_register(dst as i32, result);
        if alter_flags {
            self.set_zero_neg_flags(result);
        }
    }

    pub const fn mvn(&mut self, dst: u32, operand: u32, alter_flags: bool) {
        self.set_register(dst as i32, !operand);
        if alter_flags {
            self.set_zero_neg_flags(!operand);
        }
    }

    pub const fn mrs(&mut self, instruction: u32) {
        let using_cpsr = (instruction & (1 << 22)) == 0;
        let dst = (instruction >> 12) & 0xF;

        if using_cpsr {
            self.set_register(dst as i32, self.cpsr.get());
        } else {
            self.set_register(dst as i32, self.spsr[self.cpsr.mode as usize].get());
        }
    }

    pub fn msr(&mut self, instruction: u32) {
        let is_imm = (instruction & (1 << 25)) != 0;
        let using_cpsr = (instruction & (1 << 22)) == 0;
        let mode = self.cpsr.mode;

        let mut value = if using_cpsr {
            self.cpsr.get()
        } else {
            self.spsr[mode as usize].get()
        };

        let source = if is_imm {
            let s = instruction & 0xFF;
            let shift = (instruction & 0xF00) >> 7;
            self.rotr32(s, shift, false)
        } else {
            self.get_register((instruction & 0xF) as i32)
        };

        let mut bitmask: u32 = 0;

        if (instruction & (1 << 19)) != 0 {
            bitmask |= 0xFF000000;
        }

        if (instruction & (1 << 16)) != 0 {
            bitmask |= 0xFF;
        }

        if self.cpsr.mode == PsrMode::User {
            bitmask &= 0xFFFFFF00;
        }

        if using_cpsr {
            bitmask &= 0xFFFFFFDF;
        }

        value &= !bitmask;
        value |= source & bitmask;

        if using_cpsr {
            if let Some(mode) = PsrMode::from_u32(value & 0x1F) {
                self.update_reg_mode(mode);
            } else {
                #[cfg(feature = "tracing")]
                tracing::warn!("Invalid PsrMode: new_cpsr = {new_cpsr}");
            }
        }

        let psr: &mut PsrFlags = if using_cpsr {
            &mut self.cpsr
        } else {
            &mut self.spsr[mode as usize]
        };
        psr.set(value);
    }

    pub const fn set_zero(&mut self, cond: bool) {
        self.cpsr.zero = cond;
    }

    pub const fn set_neg(&mut self, cond: bool) {
        self.cpsr.negative = cond;
    }

    /// Flag helpers
    pub const fn set_zero_neg_flags(&mut self, value: u32) {
        self.cpsr.zero = value == 0;
        self.cpsr.negative = (value & (1 << 31)) != 0;
    }

    pub const fn set_cv_add_flags(&mut self, a: u32, b: u32, result: u32) {
        self.cpsr.carry = (0xFFFFFFFF - a) < b;
        self.cpsr.overflow = add_overflow(a, b, result);
    }

    pub const fn set_cv_sub_flags(&mut self, a: u32, b: u32, result: u32) {
        self.cpsr.carry = a >= b;
        self.cpsr.overflow = sub_overflow(a, b, result);
    }

    pub fn spsr_to_cpsr(&mut self) {
        let new_cpsr = self.spsr[self.cpsr.mode as usize].get();

        if let Some(mode) = PsrMode::from_u32(new_cpsr & 0x1F) {
            self.update_reg_mode(mode);
            self.cpsr.set(new_cpsr);
        } else {
            #[cfg(feature = "tracing")]
            tracing::warn!("Invalid PsrMode: new_cpsr = {new_cpsr}");
        }
    }

    /// Shifts and rotates
    pub const fn lsl(&mut self, value: u32, shift: i32, alter_flags: bool) -> u32 {
        if shift == 0 {
            if alter_flags {
                self.set_zero_neg_flags(value);
            }
            return value;
        }

        if shift > 31 {
            if alter_flags {
                self.set_zero_neg_flags(0);
                self.cpsr.carry = (value & (1 << 0)) != 0;
            }
            return 0;
        }

        let result = value << shift;
        if alter_flags {
            self.set_zero_neg_flags(result);
            self.cpsr.carry = (value & (1 << (32 - shift))) != 0;
        }

        value << shift
    }
    pub fn lsl_32(&mut self, _value: u32, _flags: bool) -> u32 {
        todo!()
    }

    pub const fn lsr(&mut self, value: u32, shift: i32, alter_flags: bool) -> u32 {
        if shift > 31 {
            return self.lsr_32(value, alter_flags);
        }
        let result = value >> shift;
        if alter_flags {
            self.set_zero_neg_flags(result);
            if shift > 0 {
                self.cpsr.carry = (value & (1 << (shift - 1))) != 0;
            }
        }
        result
    }
    pub const fn lsr_32(&mut self, value: u32, alter_flags: bool) -> u32 {
        if alter_flags {
            self.set_zero_neg_flags(0);
            self.cpsr.carry = (value & (1 << 31)) != 0;
        }
        0
    }

    pub const fn asr(&mut self, value: u32, shift: i32, alter_flags: bool) -> u32 {
        if shift > 31 {
            return self.asr_32(value, alter_flags);
        }
        let result = value >> shift;
        if alter_flags {
            self.set_zero_neg_flags(result);
            if shift > 0 {
                self.cpsr.carry = (value & (1 << (shift - 1))) != 0;
            }
        }
        result
    }

    pub const fn asr_32(&mut self, value: u32, alter_flags: bool) -> u32 {
        let result = value >> 31;
        if alter_flags {
            self.set_zero_neg_flags(result);
            self.cpsr.carry = (value & (1 << 31)) != 0;
        }
        result
    }

    pub const fn rrx(&mut self, value: u32, alter_flags: bool) -> u32 {
        let mut result = value;
        result >>= 1;
        if self.cpsr.carry {
            result |= 1 << 31;
        } else {
            result |= 0;
        };

        if alter_flags {
            self.set_zero_neg_flags(result);
            self.cpsr.carry = (value & 0x1) != 0;
        }
        result
    }

    pub const fn rotr32(&mut self, n: u32, mut c: u32, alter_flags: bool) -> u32 {
        const MASK: u32 = 0x1F;

        if alter_flags && (c > 0) {
            self.cpsr.carry = (n & (1 << (c - 1))) != 0;
        };
        c &= MASK;

        let neg_c = -(c as i32);
        let result = (n >> c) | (n << (neg_c & MASK as i32));

        if alter_flags {
            self.set_zero_neg_flags(result);
        }

        result
    }
}
