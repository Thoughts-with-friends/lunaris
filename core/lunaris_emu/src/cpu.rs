/// ARM CPU core implementation for both ARM7 and ARM9 processors
/// Handles instruction execution, registers, and CPU state management
use std::fmt;

/// Register constants
pub const REG_SP: usize = 13;
pub const REG_LR: usize = 14;
pub const REG_PC: usize = 15;

/// CPU processor mode enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PSRMode {
    /// User mode
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

impl PSRMode {
    /// Convert numeric value to PSRMode
    pub fn from_value(val: u32) -> Self {
        match val & 0x1F {
            0x10 => PSRMode::User,
            0x11 => PSRMode::Fiq,
            0x12 => PSRMode::Irq,
            0x13 => PSRMode::Supervisor,
            0x17 => PSRMode::Abort,
            0x1B => PSRMode::Undefined,
            0x1F => PSRMode::System,
            _ => PSRMode::User,
        }
    }
}

/// CPSR (Current Program Status Register) flags
#[derive(Debug, Clone, Copy)]
pub struct PSRFlags {
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
    pub mode: PSRMode,
}

impl PSRFlags {
    /// Create new PSR flags with default values
    pub fn new() -> Self {
        PSRFlags {
            negative: false,
            zero: false,
            carry: false,
            overflow: false,
            sticky_overflow: false,
            irq_disabled: false,
            fiq_disabled: false,
            thumb_on: false,
            mode: PSRMode::Supervisor,
        }
    }

    /// Get the PSR as a 32-bit value
    pub fn get(&self) -> u32 {
        let mut value = 0u32;

        if self.negative {
            value |= 1 << 31;
        }
        if self.zero {
            value |= 1 << 30;
        }
        if self.carry {
            value |= 1 << 29;
        }
        if self.overflow {
            value |= 1 << 28;
        }
        if self.sticky_overflow {
            value |= 1 << 27;
        }
        if self.irq_disabled {
            value |= 1 << 7;
        }
        if self.fiq_disabled {
            value |= 1 << 6;
        }
        if self.thumb_on {
            value |= 1 << 5;
        }

        value |= (self.mode as u32) & 0x1F;

        value
    }

    /// Set the PSR from a 32-bit value
    pub fn set(&mut self, value: u32) {
        self.negative = (value & (1 << 31)) != 0;
        self.zero = (value & (1 << 30)) != 0;
        self.carry = (value & (1 << 29)) != 0;
        self.overflow = (value & (1 << 28)) != 0;
        self.sticky_overflow = (value & (1 << 27)) != 0;
        self.irq_disabled = (value & (1 << 7)) != 0;
        self.fiq_disabled = (value & (1 << 6)) != 0;
        self.thumb_on = (value & (1 << 5)) != 0;
        self.mode = PSRMode::from_value(value & 0x1F);
    }
}

impl Default for PSRFlags {
    fn default() -> Self {
        Self::new()
    }
}

/// ARM processor core
/// Executes ARM and Thumb instructions for Nintendo DS emulation
pub struct ARMCPU {
    /// Reference to parent emulator
    emulator_ptr: *mut crate::emulator::Emulator,
    /// CP15 system control coprocessor
    cp15: Option<Box<dyn std::any::Any>>, // Placeholder for CP15 implementation
    /// CPU identifier (7 for ARM7, 9 for ARM9)
    cpu_id: u32,
    /// CPU halted state
    halted: bool,

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

    /// FIQ-specific registers (5 banked registers)
    fiq_regs: [u32; 5],

    /// General purpose registers (16 registers: R0-R15)
    regs: [u32; 16],

    /// Current Program Status Register
    cpsr: PSRFlags,
    /// Saved Program Status Registers (mode-specific)
    spsr: [PSRFlags; 0x20],

    /// Exception base address (for exception table)
    exception_base: u32,

    /// Current cycle timestamp
    timestamp: u64,
    /// Timestamp of last cycle
    last_timestamp: u64,
    /// Currently executing instruction
    current_instr: u32,

    /// Waitstate lookup tables for code/data access
    /// Organized by memory region and access type
    /// Format: [region][access_type]
    /// Access types: 0=n32, 1=s32, 2=n16, 3=s16
    code_waitstates: [[i32; 4]; 16],
    data_waitstates: [[i32; 4]; 16],
}

impl ARMCPU {
    /// Create a new ARM CPU instance
    pub fn new(cpu_id: u32) -> Self {
        ARMCPU {
            emulator_ptr: std::ptr::null_mut(),
            cp15: None,
            cpu_id,
            halted: false,

            sp_svc: 0,
            sp_irq: 0,
            sp_fiq: 0,
            sp_abt: 0,
            sp_und: 0,

            lr_svc: 0,
            lr_irq: 0,
            lr_fiq: 0,
            lr_abt: 0,
            lr_und: 0,

            fiq_regs: [0u32; 5],

            regs: [0u32; 16],

            cpsr: PSRFlags::new(),
            spsr: [PSRFlags::new(); 0x20],

            exception_base: 0,

            timestamp: 0,
            last_timestamp: 0,
            current_instr: 0,

            code_waitstates: [[0i32; 4]; 16],
            data_waitstates: [[0i32; 4]; 16],
        }
    }

    /// Power on the CPU
    pub fn power_on(&mut self) {
        self.halted = false;
        self.regs[REG_PC] = 0;
        self.cpsr = PSRFlags::new();
        self.cpsr.mode = PSRMode::Supervisor;
        self.cpsr.irq_disabled = true;
        self.cpsr.fiq_disabled = true;
    }

    /// Direct boot (skip BIOS) to entry point
    pub fn direct_boot(&mut self, entry_point: u32) {
        self.power_on();
        self.regs[REG_PC] = entry_point;
    }

    /// Execute the next instruction cycle
    pub fn execute(&mut self) {
        if self.halted {
            return;
        }

        // Instruction fetch and execute will be implemented with actual CPU instructions
        // For now, increment timestamp
        self.timestamp += 1;
    }

    /// Run the CPU for one cycle
    pub fn run(&mut self) {
        self.execute();
    }

    /// Jump to a new address, optionally switching Thumb mode
    pub fn jump(&mut self, new_addr: u32, change_thumb_state: bool) {
        self.regs[REG_PC] = new_addr;
        if change_thumb_state {
            self.cpsr.thumb_on = !self.cpsr.thumb_on;
        }
    }

    /// Handle undefined instruction exception
    pub fn handle_undefined(&mut self) -> Result<(), String> {
        self.cpsr.mode = PSRMode::Undefined;
        Ok(())
    }

    /// Handle IRQ interrupt
    pub fn handle_irq(&mut self) -> Result<(), String> {
        self.cpsr.mode = PSRMode::Irq;
        self.cpsr.irq_disabled = true;
        Ok(())
    }

    /// Halt the CPU
    pub fn halt(&mut self) {
        self.halted = true;
    }

    /// Handle software interrupt (SWI/SVC)
    pub fn handle_swi(&mut self) -> Result<(), String> {
        self.cpsr.mode = PSRMode::Supervisor;
        Ok(())
    }

    /// Print CPU state information for debugging
    pub fn print_info(&self) {
        println!("CPU {} State:", self.cpu_id);
        println!("  PC: 0x{:08X}", self.regs[REG_PC]);
        println!("  SP: 0x{:08X}", self.regs[REG_SP]);
        println!("  LR: 0x{:08X}", self.regs[REG_LR]);
        println!("  CPSR: 0x{:08X}", self.cpsr.get());
        println!("  Timestamp: {}", self.timestamp);
    }

    /// Update mode-specific registers when switching modes
    pub fn update_reg_mode(&mut self, _new_mode: PSRMode) {
        // Switch banked registers based on new mode
    }

    // Getters

    /// Get CPU identifier
    pub fn get_id(&self) -> u32 {
        self.cpu_id
    }

    /// Get current timestamp
    pub fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    /// Get cycles executed since last reset
    pub fn cycles_ran(&self) -> i64 {
        (self.timestamp - self.last_timestamp) as i64
    }

    /// Get program counter
    pub fn get_pc(&self) -> u32 {
        self.regs[REG_PC]
    }

    /// Get currently executing instruction
    pub fn get_current_instr(&self) -> u32 {
        self.current_instr
    }

    /// Get general purpose register value
    pub fn get_register(&self, id: usize) -> u32 {
        if id < 16 {
            self.regs[id]
        } else {
            0
        }
    }

    /// Get CPSR (Current Program Status Register)
    pub fn get_cpsr(&self) -> PSRFlags {
        self.cpsr
    }

    // Setters

    /// Set general purpose register value
    pub fn set_register(&mut self, id: usize, value: u32) {
        if id < 16 {
            self.regs[id] = value;
        }
    }

    /// Set CPSR value
    pub fn set_cpsr(&mut self, value: u32) {
        self.cpsr.set(value);
    }

    // Memory Access

    /// Read 32-bit word from memory
    pub fn read_word(&mut self, address: u32) -> u32 {
        // Will use emulator's memory access methods
        0
    }

    /// Read 16-bit halfword from memory
    pub fn read_halfword(&mut self, address: u32) -> u16 {
        // Will use emulator's memory access methods
        0
    }

    /// Read 8-bit byte from memory
    pub fn read_byte(&mut self, address: u32) -> u8 {
        // Will use emulator's memory access methods
        0
    }

    /// Write 32-bit word to memory
    pub fn write_word(&mut self, address: u32, word: u32) -> Result<(), String> {
        // Will use emulator's memory access methods
        Ok(())
    }

    /// Write 16-bit halfword to memory
    pub fn write_halfword(&mut self, address: u32, halfword: u16) -> Result<(), String> {
        // Will use emulator's memory access methods
        Ok(())
    }

    /// Write 8-bit byte to memory
    pub fn write_byte(&mut self, address: u32, byte: u8) -> Result<(), String> {
        // Will use emulator's memory access methods
        Ok(())
    }

    // Waitstate management

    /// Add non-sequential 32-bit code fetch waitstates
    pub fn add_n32_code(&mut self, address: u32, cycles: i32) {
        let region = ((address >> 24) & 0xF) as usize;
        if region < 16 {
            self.code_waitstates[region][0] += cycles;
        }
    }

    /// Add sequential 32-bit code fetch waitstates
    pub fn add_s32_code(&mut self, address: u32, cycles: i32) {
        let region = ((address >> 24) & 0xF) as usize;
        if region < 16 {
            self.code_waitstates[region][1] += cycles;
        }
    }

    /// Add non-sequential 16-bit code fetch waitstates
    pub fn add_n16_code(&mut self, address: u32, cycles: i32) {
        let region = ((address >> 24) & 0xF) as usize;
        if region < 16 {
            self.code_waitstates[region][2] += cycles;
        }
    }

    /// Add sequential 16-bit code fetch waitstates
    pub fn add_s16_code(&mut self, address: u32, cycles: i32) {
        let region = ((address >> 24) & 0xF) as usize;
        if region < 16 {
            self.code_waitstates[region][3] += cycles;
        }
    }

    /// Add non-sequential 32-bit data access waitstates
    pub fn add_n32_data(&mut self, address: u32, cycles: i32) {
        let region = ((address >> 24) & 0xF) as usize;
        if region < 16 {
            self.data_waitstates[region][0] += cycles;
        }
    }

    /// Add sequential 32-bit data access waitstates
    pub fn add_s32_data(&mut self, address: u32, cycles: i32) {
        let region = ((address >> 24) & 0xF) as usize;
        if region < 16 {
            self.data_waitstates[region][1] += cycles;
        }
    }

    /// Add non-sequential 16-bit data access waitstates
    pub fn add_n16_data(&mut self, address: u32, cycles: i32) {
        let region = ((address >> 24) & 0xF) as usize;
        if region < 16 {
            self.data_waitstates[region][2] += cycles;
        }
    }

    /// Add sequential 16-bit data access waitstates
    pub fn add_s16_data(&mut self, address: u32, cycles: i32) {
        let region = ((address >> 24) & 0xF) as usize;
        if region < 16 {
            self.data_waitstates[region][3] += cycles;
        }
    }

    /// Add internal processing cycles
    pub fn add_internal_cycles(&mut self, cycles: i32) {
        self.timestamp += cycles as u64;
    }

    /// Add coprocessor operation cycles
    pub fn add_cop_cycles(&mut self, cycles: i32) {
        self.timestamp += cycles as u64;
    }

    // Arithmetic and Logical Operations

    /// Bitwise AND operation
    pub fn and(&mut self, dest: usize, src: usize, operand: u32, set_flags: bool) {
        let result = self.regs[src] & operand;
        self.regs[dest] = result;
        if set_flags {
            self.set_zero_neg_flags(result);
        }
    }

    /// Bitwise OR operation
    pub fn orr(&mut self, dest: usize, src: usize, operand: u32, set_flags: bool) {
        let result = self.regs[src] | operand;
        self.regs[dest] = result;
        if set_flags {
            self.set_zero_neg_flags(result);
        }
    }

    /// Bitwise XOR operation
    pub fn eor(&mut self, dest: usize, src: usize, operand: u32, set_flags: bool) {
        let result = self.regs[src] ^ operand;
        self.regs[dest] = result;
        if set_flags {
            self.set_zero_neg_flags(result);
        }
    }

    /// Add operation
    pub fn add(&mut self, dest: usize, src: usize, operand: u32, set_flags: bool) {
        let result = self.regs[src].wrapping_add(operand);
        self.regs[dest] = result;
        if set_flags {
            self.set_cv_add_flags(self.regs[src], operand, result);
            self.set_zero_neg_flags(result);
        }
    }

    /// Subtract operation
    pub fn sub(&mut self, dest: usize, src: usize, operand: u32, set_flags: bool) {
        let result = self.regs[src].wrapping_sub(operand);
        self.regs[dest] = result;
        if set_flags {
            self.set_cv_sub_flags(self.regs[src], operand, result);
            self.set_zero_neg_flags(result);
        }
    }

    /// Add with carry
    pub fn adc(&mut self, dest: usize, src: usize, operand: u32, set_flags: bool) {
        let carry_in = if self.cpsr.carry { 1 } else { 0 };
        let result = self.regs[src].wrapping_add(operand).wrapping_add(carry_in);
        self.regs[dest] = result;
        if set_flags {
            self.set_cv_add_flags(self.regs[src], operand.wrapping_add(carry_in), result);
            self.set_zero_neg_flags(result);
        }
    }

    /// Subtract with carry
    pub fn sbc(&mut self, dest: usize, src: usize, operand: u32, set_flags: bool) {
        let carry_in = if self.cpsr.carry { 0 } else { 1 };
        let result = self.regs[src].wrapping_sub(operand).wrapping_sub(carry_in);
        self.regs[dest] = result;
        if set_flags {
            self.set_cv_sub_flags(self.regs[src], operand.wrapping_add(carry_in), result);
            self.set_zero_neg_flags(result);
        }
    }

    /// Compare operation (sets flags without storing result)
    pub fn cmp(&mut self, x: u32, y: u32) {
        let result = x.wrapping_sub(y);
        self.set_cv_sub_flags(x, y, result);
        self.set_zero_neg_flags(result);
    }

    /// Compare negative (CMN)
    pub fn cmn(&mut self, x: u32, y: u32) {
        let result = x.wrapping_add(y);
        self.set_cv_add_flags(x, y, result);
        self.set_zero_neg_flags(result);
    }

    /// Test bits (TST)
    pub fn tst(&mut self, x: u32, y: u32) {
        let result = x & y;
        self.set_zero_neg_flags(result);
    }

    /// Test equivalence (TEQ)
    pub fn teq(&mut self, x: u32, y: u32) {
        let result = x ^ y;
        self.set_zero_neg_flags(result);
    }

    /// Move operation
    pub fn mov(&mut self, dest: usize, operand: u32, alter_flags: bool) {
        self.regs[dest] = operand;
        if alter_flags {
            self.set_zero_neg_flags(operand);
        }
    }

    /// Multiply operation
    pub fn mul(&mut self, dest: usize, src: usize, operand: u32, set_flags: bool) {
        let result = self.regs[src].wrapping_mul(operand);
        self.regs[dest] = result;
        if set_flags {
            self.set_zero_neg_flags(result);
        }
    }

    /// Bit clear operation (BIC)
    pub fn bic(&mut self, dest: usize, src: usize, operand: u32, alter_flags: bool) {
        let result = self.regs[src] & !operand;
        self.regs[dest] = result;
        if alter_flags {
            self.set_zero_neg_flags(result);
        }
    }

    /// Move NOT operation (MVN)
    pub fn mvn(&mut self, dest: usize, operand: u32, alter_flags: bool) {
        let result = !operand;
        self.regs[dest] = result;
        if alter_flags {
            self.set_zero_neg_flags(result);
        }
    }

    /// Move from status register (MRS)
    pub fn mrs(&mut self, _instruction: u32) {
        // Read CPSR or SPSR
    }

    /// Move to status register (MSR)
    pub fn msr(&mut self, _instruction: u32) {
        // Write CPSR or SPSR
    }

    // Flag operations

    /// Set zero flag based on condition
    pub fn set_zero(&mut self, cond: bool) {
        self.cpsr.zero = cond;
    }

    /// Set negative flag based on condition
    pub fn set_neg(&mut self, cond: bool) {
        self.cpsr.negative = cond;
    }

    /// Set zero and negative flags based on value
    pub fn set_zero_neg_flags(&mut self, value: u32) {
        self.cpsr.zero = value == 0;
        self.cpsr.negative = (value & (1 << 31)) != 0;
    }

    /// Set carry and overflow flags for addition
    fn set_cv_add_flags(&mut self, a: u32, b: u32, result: u32) {
        // Carry: (0xFFFFFFFF - a) < b
        self.cpsr.carry = (0xFFFFFFFFu32.wrapping_sub(a)) < b;
        // Overflow: result sign differs from operand signs when operands have same sign
        let same_sign = ((a ^ b) & 0x80000000) == 0;
        let result_diff_sign = ((a ^ result) & 0x80000000) != 0;
        self.cpsr.overflow = same_sign && result_diff_sign;
    }

    /// Set carry and overflow flags for subtraction
    fn set_cv_sub_flags(&mut self, a: u32, b: u32, result: u32) {
        // Carry: a >= b (no borrow)
        self.cpsr.carry = a >= b;
        // Overflow: operands have different signs and result sign differs from first operand
        let diff_sign = ((a ^ b) & 0x80000000) != 0;
        let result_diff_sign = ((a ^ result) & 0x80000000) != 0;
        self.cpsr.overflow = diff_sign && result_diff_sign;
    }

    /// Move SPSR to CPSR
    pub fn spsr_to_cpsr(&mut self) {
        let mode_index = (self.cpsr.mode as u32) as usize;
        if mode_index < 0x20 {
            self.cpsr = self.spsr[mode_index];
        }
    }

    // Shift operations

    /// Logical shift left
    pub fn lsl(&mut self, value: u32, shift: u32, alter_flags: bool) -> u32 {
        if shift == 0 {
            value
        } else if shift >= 32 {
            if alter_flags {
                self.cpsr.carry = false;
            }
            0
        } else {
            if alter_flags {
                self.cpsr.carry = (value & (1 << (32 - shift))) != 0;
            }
            value.wrapping_shl(shift)
        }
    }

    /// Logical shift right
    pub fn lsr(&mut self, value: u32, shift: u32, alter_flags: bool) -> u32 {
        if shift == 0 {
            value
        } else if shift >= 32 {
            if alter_flags {
                self.cpsr.carry = false;
            }
            0
        } else {
            if alter_flags {
                self.cpsr.carry = (value & (1 << (shift - 1))) != 0;
            }
            value.wrapping_shr(shift)
        }
    }

    /// Arithmetic shift right
    pub fn asr(&mut self, value: u32, shift: u32, alter_flags: bool) -> u32 {
        let sign_bit = value & 0x80000000;
        if shift == 0 {
            value
        } else if shift >= 32 {
            if alter_flags {
                self.cpsr.carry = sign_bit != 0;
            }
            if sign_bit != 0 {
                0xFFFFFFFF
            } else {
                0
            }
        } else {
            if alter_flags {
                self.cpsr.carry = (value & (1 << (shift - 1))) != 0;
            }
            (value as i32).wrapping_shr(shift) as u32
        }
    }

    /// Rotate right with extend
    pub fn rrx(&mut self, value: u32, alter_flags: bool) -> u32 {
        let carry_in = if self.cpsr.carry { 1u32 << 31 } else { 0 };
        if alter_flags {
            self.cpsr.carry = (value & 1) != 0;
        }
        (value >> 1) | carry_in
    }

    /// Rotate right by amount
    pub fn rotr32(&mut self, n: u32, c: u32, alter_flags: bool) -> u32 {
        let c = c & 31;
        if alter_flags && c > 0 {
            self.cpsr.carry = (n & (1 << (c - 1))) != 0;
        }
        n.rotate_right(c)
    }
}

impl fmt::Debug for ARMCPU {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ARMCPU")
            .field("cpu_id", &self.cpu_id)
            .field("halted", &self.halted)
            .field("timestamp", &self.timestamp)
            .field("regs", &self.regs)
            .field("cpsr", &self.cpsr)
            .finish()
    }
}
