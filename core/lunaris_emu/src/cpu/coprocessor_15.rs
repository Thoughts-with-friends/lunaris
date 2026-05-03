//! ARM9 Coprocessor 15 (CP15) - System Control Coprocessor
//! Manages ARM9 memory protection unit, caches, TCM, and control registers

use lunaris_ds_mem_const::{DTCM_MASK, ITCM_MASK};

/// CP15 Control Register
#[derive(Debug, Clone, Copy)]
pub struct ControlReg {
    /// MMU/Protection Unit enable
    pub mmu_pu_enable: bool,
    /// Data/Unified cache enable
    pub data_unified_cache_on: bool,
    /// Big endian mode (vs little endian)
    pub is_big_endian: bool,
    /// Instruction cache enable
    pub instruction_cache_on: bool,
    /// High exception vector (0x0000 vs 0xFFFF0000)
    pub high_exception_vector: bool,
    /// Predictable cache replacement algorithm
    pub predictable_cache_replacement: bool,
    /// Pre-ARMv5 mode compatibility
    pub pre_armv5_mode: bool,
    /// Data TCM enable
    pub dtcm_enable: bool,
    /// Data TCM write-only mode
    pub dtcm_write_only: bool,
    /// Instruction TCM enable
    pub itcm_enable: bool,
    /// Instruction TCM write-only mode
    pub itcm_write_only: bool,
}

impl ControlReg {
    /// Create new control register with defaults
    pub const fn new() -> Self {
        Self {
            // control.set_values(0);
            mmu_pu_enable: false,
            data_unified_cache_on: false,
            is_big_endian: false,
            instruction_cache_on: false,
            high_exception_vector: false,
            predictable_cache_replacement: false,
            pre_armv5_mode: false,
            dtcm_enable: false,
            dtcm_write_only: false,
            itcm_enable: false,
            itcm_write_only: false,
        }
    }

    /// Get control register value as 32-bit word
    pub const fn get_values(&self) -> u32 {
        let mut value = 0;
        if self.mmu_pu_enable {
            value |= 1 << 0;
        }
        if self.data_unified_cache_on {
            value |= 1 << 2;
        }
        if self.is_big_endian {
            value |= 1 << 7;
        }
        if self.instruction_cache_on {
            value |= 1 << 12;
        }
        if self.high_exception_vector {
            value |= 1 << 13;
        }
        if self.predictable_cache_replacement {
            value |= 1 << 14;
        }
        if self.pre_armv5_mode {
            value |= 1 << 15;
        }
        if self.dtcm_enable {
            value |= 1 << 16;
        }
        if self.dtcm_write_only {
            value |= 1 << 17;
        }
        if self.itcm_enable {
            value |= 1 << 18;
        }
        if self.itcm_write_only {
            value |= 1 << 19;
        }
        value
    }

    /// Set control register from 32-bit word
    pub const fn set_values(&mut self, reg: u32) {
        self.mmu_pu_enable = (reg & (1 << 0)) != 0;
        self.data_unified_cache_on = (reg & (1 << 2)) != 0;
        self.is_big_endian = (reg & (1 << 7)) != 0;
        self.instruction_cache_on = (reg & (1 << 12)) != 0;
        self.high_exception_vector = (reg & (1 << 13)) != 0;
        self.predictable_cache_replacement = (reg & (1 << 14)) != 0;
        self.pre_armv5_mode = (reg & (1 << 15)) != 0;
        self.dtcm_enable = (reg & (1 << 16)) != 0;
        self.dtcm_write_only = (reg & (1 << 17)) != 0;
        self.itcm_enable = (reg & (1 << 18)) != 0;
        self.itcm_write_only = (reg & (1 << 19)) != 0;
    }
}

impl Default for ControlReg {
    fn default() -> Self {
        Self::new()
    }
}

/// ARM9 Coprocessor 15 System Control
/// Manages instruction/data TCM, caches, and memory control
#[derive(Debug)]
pub struct Cp15 {
    /// Control register
    control: ControlReg,

    /// Instruction TCM data register
    itcm_data: u32,
    /// Data TCM data register
    dtcm_data: u32,

    /// Instruction TCM size (cached value)
    pub itcm_size: u32,
    /// Data TCM base address (cached value)
    pub dtcm_base: u32,
    /// Data TCM size (cached value)
    pub dtcm_size: u32,

    /// Instruction TCM memory (32 KB)
    itcm: Vec<u8>,
    /// Data TCM memory (16 KB)
    dtcm: Vec<u8>,

    /// Data cache (4 KB)
    dcache: Vec<u8>,
    /// Instruction cache (8 KB)
    icache: Vec<u8>,
}

impl Default for Cp15 {
    fn default() -> Self {
        Self::new()
    }
}

impl Cp15 {
    const ITCM_SIZE: u32 = 0x0200_0000;
    const DTCM_BASE: u32 = 0x0B00_0000;
    const DTCM_SIZE: u32 = 16 * 1024;

    /// Create new CP15 controller
    pub fn new() -> Self {
        Self {
            // Doesn't reflect true initial values, but does give us a basis when executing the BIOS
            control: ControlReg::new(),
            itcm_data: 0,
            dtcm_data: 0,
            itcm_size: Self::ITCM_SIZE,
            dtcm_base: Self::DTCM_BASE,
            dtcm_size: 0x00010000, // NOTE: 64 KB, but only 16 KB is actually used by ARM9
            itcm: vec![0_u8; 1024 * 32],
            dtcm: vec![0_u8; 1024 * 16],
            dcache: vec![0_u8; 1024 * 4],
            icache: vec![0_u8; 1024 * 8],
        }
    }

    /// Power on CP15
    pub const fn power_on(&mut self) {
        self.control = ControlReg {
            mmu_pu_enable: false,
            data_unified_cache_on: false,
            is_big_endian: false,
            instruction_cache_on: false,
            high_exception_vector: false,
            predictable_cache_replacement: false,
            pre_armv5_mode: false,
            dtcm_enable: false,
            dtcm_write_only: false,
            itcm_enable: true,
            itcm_write_only: true,
        };
        self.itcm_size = Self::ITCM_SIZE;
        self.dtcm_base = Self::DTCM_BASE;
        self.dtcm_size = Self::DTCM_SIZE;
    }

    // Removed
    //
    // Link CP15 with ARM9 CPU
    // fn link_with_cpu(&mut self);

    /// Get instruction TCM size
    /// Returns size in bytes if enabled, 0 if disabled
    pub const fn get_itcm_size(&self) -> u32 {
        if self.control.itcm_enable {
            self.itcm_size
        } else {
            0
        }
    }

    /// Get data TCM base address
    pub const fn get_dtcm_base(&self) -> u32 {
        self.dtcm_base
    }

    /// Get data TCM size
    /// Returns size in bytes if enabled, 0 if disabled
    pub const fn get_dtcm_size(&self) -> u32 {
        if self.control.dtcm_enable {
            self.dtcm_size
        } else {
            0
        }
    }

    /// Check if data TCM is write-only
    pub const fn dtcm_write_only(&self) -> bool {
        self.control.dtcm_write_only
    }

    /// Read word from TCM/cache
    pub fn read_word(&self, address: u32) -> u32 {
        self.read_byte(address) as u32
            | ((self.read_byte(address + 1) as u32) << 8)
            | ((self.read_byte(address + 2) as u32) << 16)
            | ((self.read_byte(address + 3) as u32) << 24)
    }

    /// Read halfword from TCM/cache
    pub fn read_halfword(&self, address: u32) -> u16 {
        (self.read_byte(address) as u16) | ((self.read_byte(address + 1) as u16) << 8)
    }

    /// Read byte from TCM/cache
    pub fn read_byte(&self, address: u32) -> u8 {
        let dtcm_size = self.dtcm_size;
        let dtcm_base = self.dtcm_base;

        if address < dtcm_size {
            self.itcm[(address & ITCM_MASK) as usize]
        } else if address >= dtcm_base && address < (dtcm_base + dtcm_size) {
            self.dtcm[(address & DTCM_MASK) as usize]
        } else {
            // self.arm9_read_byte(address, byte);
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Unreachable: Since `struct emulator` is inaccessible due to a circular reference issue, it is called at a higher level."
            );
            0x0
        }
    }

    /// Write word to TCM/cache
    pub fn write_word(&mut self, address: u32, word: u32) {
        self.write_byte(address, word & 0xFF);
        self.write_byte(address + 1, (word >> 8) & 0xFF);
        self.write_byte(address + 2, (word >> 16) & 0xFF);
        self.write_byte(address + 3, (word >> 24) & 0xFF);
    }

    /// Write halfword to TCM/cache
    pub fn write_halfword(&mut self, address: u32, halfword: u16) {
        // <---------->
        // [0_u8, 1_u8]
        // TODO: or unsafe { core::mem::transmute(src) }
        self.write_byte(address, (halfword & 0xFF) as u32);
        self.write_byte(address + 1, ((halfword >> 8) & 0xFF) as u32);
    }

    /// Write byte to TCM/cache
    pub fn write_byte(&mut self, address: u32, byte: u32) {
        let dtcm_size = self.dtcm_size;
        let dtcm_base = self.dtcm_base;

        if address < dtcm_size {
            self.itcm[(address & ITCM_MASK) as usize] = byte as u8;
        } else if address >= dtcm_base && address < (dtcm_base + dtcm_size) {
            self.dtcm[(address & DTCM_MASK) as usize] = byte as u8;
        } else {
            // self.arm9_write_byte(address, byte);
            #[cfg(feature = "tracing")]
            tracing::error!(
                "Unreachable: Since `struct emulator` is inaccessible due to a circular reference issue, it is called at a higher level."
            );
        }
    }

    /// MRC instruction - Read from coprocessor
    /// Moves coprocessor register to ARM register
    pub const fn mrc(&self, operation: i32, source_reg: i32, _info: i32, operand_reg: i32) -> u32 {
        match (operation, source_reg, operand_reg) {
            (0, 0, 0) => self.control.get_values(), // Read control register
            _ => 0,
        }
    }

    /// MCR instruction - Write to coprocessor
    /// Moves ARM register to coprocessor register
    pub const fn mcr(
        &mut self,
        operation: i32,
        destination_reg: i32,
        arm_reg_contents: u32,
        _info: i32,
        operand_reg: i32,
    ) {
        match (operation, destination_reg, operand_reg) {
            (0, 1, 0) => {
                // Write control register
                self.control.set_values(arm_reg_contents);
            }
            (0, 9, 0) => {
                // Data TCM region
                self.dtcm_data = arm_reg_contents;
                self.dtcm_base = arm_reg_contents & 0xFFFFF000;
                self.dtcm_size = 512 << ((arm_reg_contents >> 1) & 0x1F);
            }
            (0, 9, 1) => {
                // Instruction TCM region
                self.itcm_data = arm_reg_contents;
                self.itcm_size = 512 << ((arm_reg_contents >> 1) & 0x1F);
            }
            _ => {}
        }
    }

    /// Get control register
    pub const fn get_control(&self) -> &ControlReg {
        &self.control
    }

    /// Get mutable control register
    pub const fn get_control_mut(&mut self) -> &mut ControlReg {
        &mut self.control
    }

    /// Invalidate instruction cache
    pub fn invalidate_icache(&mut self) {
        self.icache.fill(0);
    }

    /// Invalidate data cache
    pub fn invalidate_dcache(&mut self) {
        self.dcache.fill(0);
    }
}
