//! ARM CPU core (skeleton)
//!
//! Ported from CorgiDS ARM CPU header.
//! All behavior is currently unimplemented (`todo!()`).

/// General-purpose register indices
pub const REG_SP: usize = 13;
pub const REG_LR: usize = 14;
pub const REG_PC: usize = 15;

/// ARM processor modes (CPSR.M)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum PsrMode {
    #[default]
    User = 0x10,
    Fiq = 0x11,
    Irq = 0x12,
    Supervisor = 0x13,
    Abort = 0x17,
    Undefined = 0x1B,
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
    pub negative: bool,
    pub zero: bool,
    pub carry: bool,
    pub overflow: bool,
    pub sticky_overflow: bool,

    pub irq_disabled: bool,
    pub fiq_disabled: bool,
    pub thumb_on: bool,

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

/// Forward declarations
pub struct Emulator;
pub struct Cp15;

/// ARM CPU core
#[derive(Debug, Default)]
pub struct ArmCpu {
    /// CPU ID (ARM9 / ARM7 etc.)
    cpu_id: i32,

    /// Halt state
    halted: bool,

    /// Banked stack pointers
    sp_svc: u32,
    sp_irq: u32,
    sp_fiq: u32,
    sp_abt: u32,
    sp_und: u32,

    /// Banked link registers
    lr_svc: u32,
    lr_irq: u32,
    lr_fiq: u32,
    lr_abt: u32,
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
    code_waitstates: [[i32; 4]; 16],

    /// Data waitstates [region][n32/s32/n16/s16]
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
    pub fn set_cp15(&mut self, cp15: *mut Cp15) {
        todo!()
    }

    /// Reset CPU state
    pub fn power_on(&mut self) {
        todo!()
    }

    /// Boot directly to an entry point
    pub fn direct_boot(&mut self, entry_point: u32) {
        todo!()
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
    pub fn jp(&mut self, new_addr: u32, change_thumb_state: bool) {
        todo!()
    }

    /// Exception handlers
    pub fn handle_undefined(&mut self) {
        todo!()
    }
    pub fn handle_irq(&mut self) {
        todo!()
    }

    /// Halt execution
    pub fn halt(&mut self) {
        todo!()
    }
    pub fn handle_swi(&mut self) {
        todo!()
    }

    /// Debug helpers
    pub fn print_info(&self) {
        todo!()
    }

    /// Update register banking for a new CPU mode
    pub fn update_reg_mode(&mut self, new_mode: PsrMode) {
        todo!()
    }

    /// Debug helpers
    pub fn get_reg_name(id: i32) -> String {
        todo!()
    }
    pub fn get_condition_name(id: i32) -> String {
        todo!()
    }

    pub const fn get_id(&self) -> i32 {
        todo!()
    }

    /// Register access
    pub const fn get_register(&self, id: usize) -> u32 {
        self.regs[id]
    }

    pub const fn set_register(&mut self, id: usize, value: u32) {
        self.regs[id] = value;
    }

    /// Condition code evaluation
    pub fn check_condition(&self, condition: i32) -> bool {
        todo!()
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
        self.regs[REG_PC]
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

    //Waitstate bullshit
    pub fn add_n32_code(&mut self, address: u32, cycles: i32) {
        todo!()
    }
    pub fn add_s32_code(&mut self, address: u32, cycles: i32) {
        todo!()
    }
    pub fn add_n16_code(&mut self, address: u32, cycles: i32) {
        todo!()
    }
    pub fn add_s16_code(&mut self, address: u32, cycles: i32) {
        todo!()
    }
    pub fn add_n32_data(&mut self, address: u32, cycles: i32) {
        todo!()
    }
    pub fn add_s32_data(&mut self, address: u32, cycles: i32) {
        todo!()
    }
    pub fn add_n16_data(&mut self, address: u32, cycles: i32) {
        todo!()
    }
    pub fn add_s16_data(&mut self, address: u32, cycles: i32) {
        todo!()
    }

    /// Cycle accounting
    pub const fn add_internal_cycles(&mut self, cycles: i32) {
        self.timestamp += cycles as u64;
    }

    pub const fn add_cop_cycles(&mut self, cycles: i32) {
        self.timestamp += cycles as u64;
    }

    // All data manipulation methods here
    pub fn andd(&mut self, _dst: i32, _src: i32, _op: i32, _set_cc: bool) {
        todo!()
    }
    pub fn orr(&mut self, _dst: i32, _src: i32, _op: i32, _set_cc: bool) {
        todo!()
    }
    pub fn eor(&mut self, _dst: i32, _src: i32, _op: i32, _set_cc: bool) {
        todo!()
    }
    pub fn add(&mut self, _dst: u32, _src: u32, _op: u32, _set_cc: bool) {
        todo!()
    }
    pub fn sub(&mut self, _dst: u32, _src: u32, _op: u32, _set_cc: bool) {
        todo!()
    }
    pub fn adc(&mut self, _dst: u32, _src: u32, _op: u32, _set_cc: bool) {
        todo!()
    }
    pub fn sbc(&mut self, _dst: u32, _src: u32, _op: u32, _set_cc: bool) {
        todo!()
    }

    pub fn cmp(&mut self, _x: u32, _y: u32) {
        todo!()
    }
    pub fn cmn(&mut self, _x: u32, _y: u32) {
        todo!()
    }
    pub fn tst(&mut self, _x: u32, _y: u32) {
        todo!()
    }
    pub fn teq(&mut self, _x: u32, _y: u32) {
        todo!()
    }

    pub fn mov(&mut self, _dst: u32, _op: u32, _set_flags: bool) {
        todo!()
    }
    pub fn mul(&mut self, _dst: u32, _src: u32, _op: u32, _set_cc: bool) {
        todo!()
    }
    pub fn bic(&mut self, _dst: u32, _src: u32, _op: u32, _set_flags: bool) {
        todo!()
    }
    pub fn mvn(&mut self, _dst: u32, _op: u32, _set_flags: bool) {
        todo!()
    }

    pub fn mrs(&mut self, _instr: u32) {
        todo!()
    }
    pub fn msr(&mut self, _instr: u32) {
        todo!()
    }

    pub fn set_zero(&mut self, cond: bool) {
        todo!()
    }

    pub fn set_neg(&mut self, cond: bool) {
        todo!()
    }

    /// Flag helpers
    pub const fn set_zero_neg_flags(&mut self, value: u32) {
        self.cpsr.zero = value == 0;
        self.cpsr.negative = (value & (1 << 31)) != 0;
    }

    pub fn set_cv_add_flags(&mut self, a: u32, b: u32, result: u32) {
        todo!()
    }
    pub fn set_cv_sub_flags(&mut self, a: u32, b: u32, result: u32) {
        todo!()
    }

    pub fn spsr_to_cpsr(&mut self) {
        todo!()
    }

    /// Shifts and rotates
    pub fn lsl(&mut self, _value: u32, _shift: i32, _flags: bool) -> u32 {
        todo!()
    }
    pub fn lsl_32(&mut self, _value: u32, _flags: bool) -> u32 {
        todo!()
    }

    pub fn lsr(&mut self, _value: u32, _shift: i32, _flags: bool) -> u32 {
        todo!()
    }
    pub fn lsr_32(&mut self, _value: u32, _flags: bool) -> u32 {
        todo!()
    }

    pub fn asr(&mut self, _value: u32, _shift: i32, _flags: bool) -> u32 {
        todo!()
    }
    pub fn asr_32(&mut self, _value: u32, _flags: bool) -> u32 {
        todo!()
    }

    pub fn rrx(&mut self, _value: u32, _flags: bool) -> u32 {
        todo!()
    }

    pub fn rotr32(&mut self, n: u32, c: u32, _flags: bool) -> u32 {
        todo!()
    }
}
