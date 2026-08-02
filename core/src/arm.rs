//! ARM CPU core – generic over `IS_ARM9`.
//!
//! `ARM<true>`  → ARM946E-S (ARM9, runs at 2× the master clock).
//! `ARM<false>` → ARM7TDMI  (ARM7, runs at 1× the master clock).
//!
//! Both cores share the same execution loop; instruction decoding is done via
//! pre-built lookup tables ([`ArmLut`] / [`ThumbLut`]) stored in `OnceLock`
//! statics so they are generated only once per process.
//!
//! GBATEK references:
//! - CPU overview / versions: <https://problemkaputt.de/gbatek.htm#armcpuoverview>
//! - Register set & modes: <https://problemkaputt.de/gbatek.htm#armcpuregisterset>
//! - Exceptions (IRQ vector, mode switch): <https://problemkaputt.de/gbatek.htm#armcpuexceptions>
//! - Instruction cycle times: <https://problemkaputt.de/gbatek.htm#armcpuinstructioncycletimes>
//! - Clock rates (ARM9 66 MHz / ARM7 33 MHz): <https://problemkaputt.de/gbatek.htm#dstechnicaldata>

#[macro_use]
mod instructions;
mod arm;
mod registers;
mod thumb;

use std::sync::OnceLock;

use registers::{Mode, RegValues};

use crate::{
    hw::{AccessType, HW, MemoryValue},
    likely, num, unlikely,
};

/// 4096-entry ARM instruction dispatch table (bits [27:20] + [7:4] of opcode).
type ArmLut<const IS_ARM9: bool> = [instructions::InstructionHandler<u32, IS_ARM9>; 4096];
/// 256-entry THUMB instruction dispatch table (bits [15:8] of opcode).
type ThumbLut<const IS_ARM9: bool> = [instructions::InstructionHandler<u16, IS_ARM9>; 256];

/// Emulated ARM CPU core.
///
/// `IS_ARM9 = true` selects the ARM946E-S; `false` selects the ARM7TDMI.
/// The three lookup tables are static and shared across all instances of the
/// same `IS_ARM9` variant (built lazily via [`OnceLock`]).
#[derive(emu_utils::Savestate)]
pub struct ARM<const IS_ARM9: bool> {
    /// Current cycle count (ARM9 counts at 2×, ARM7 at 1×).
    ///
    /// Serialized as `u64`: emu-utils stores `usize` as `u32`, which silently
    /// truncates this absolute cycle counter after ~64s (ARM9) / ~128s (ARM7)
    /// of real play. See `docs/design/savestate-and-video-design.md` §3.
    #[store(with = "save.store(&mut (*cycle as u64))?")]
    #[load(
        with = "save.load::<u64>()? as usize",
        with_in_place = "*cycle = save.load::<u64>()? as usize"
    )]
    cycle: usize,
    regs: RegValues,
    /// Two-word prefetch pipeline buffer (`[0]` = current, `[1]` = next).
    instr_buffer: [u32; 2],
    next_access_type: AccessType,

    /// Condition-code evaluation table indexed by `(NZCV << 4) | cond`.
    #[savestate(skip)]
    condition_lut: &'static [bool; 256],
    #[savestate(skip)]
    arm_lut: &'static ArmLut<IS_ARM9>,
    #[savestate(skip)]
    thumb_lut: &'static ThumbLut<IS_ARM9>,
}

impl<const IS_ARM9: bool> ARM<IS_ARM9> {
    pub fn new(hw: &mut HW, direct_boot: bool) -> ARM<IS_ARM9> {
        let mut cpu = ARM {
            cycle: 0,
            regs: if direct_boot {
                RegValues::direct_boot::<IS_ARM9>(if IS_ARM9 {
                    hw.init_arm9()
                } else {
                    hw.init_arm7()
                })
            } else {
                RegValues::new::<IS_ARM9>()
            },
            instr_buffer: [0; 2],
            next_access_type: AccessType::N,

            condition_lut: condition_lut(),
            arm_lut: arm_lut(),
            thumb_lut: thumb_lut(),
        };

        cpu.fill_arm_instr_buffer(hw);
        cpu
    }

    pub fn set_cycle(&mut self, cycle: usize) {
        self.cycle = cycle;
    }

    /// Test-only: shifts `cycle` by `offset`, simulating a long play session
    /// for u32-overflow regression tests.
    /// See `docs/design/savestate-and-video-design.md` §3.4.
    #[cfg(test)]
    pub(crate) fn offset_cycle_for_test(&mut self, offset: usize) {
        self.cycle = self.cycle.wrapping_add(offset);
    }

    /// Executes instructions until `self.cycle >= target`.
    ///
    /// Checks for IRQs and halt state before every instruction.
    /// Dispatches to the THUMB or ARM path based on the T-bit in CPSR.
    pub fn emulate(&mut self, hw: &mut HW, target: usize) {
        while self.cycle < target {
            self.handle_irq(hw);
            if self.is_halted(hw) {
                self.cycle = target;
                return;
            }

            if unlikely(self.regs.get_t()) {
                self.emulate_thumb_instr(hw)
            } else {
                self.emulate_arm_instr(hw)
            }
        }
    }

    pub fn read<T: MemoryValue>(&mut self, hw: &mut HW, access_type: AccessType, addr: u32) -> T {
        let value = if IS_ARM9 {
            let value = hw.arm9_read::<T>(addr);
            self.cycle += hw.arm9_get_access_time::<T>(self.next_access_type, addr);
            value
        } else {
            let value = hw.arm7_read::<T>(addr);
            self.cycle += hw.arm7_get_access_time::<T>(self.next_access_type, addr);
            value
        };
        self.next_access_type = access_type;
        value
    }

    pub fn write<T: MemoryValue>(
        &mut self,
        hw: &mut HW,
        access_type: AccessType,
        addr: u32,
        value: T,
    ) {
        if IS_ARM9 {
            self.cycle += hw.arm9_get_access_time::<T>(self.next_access_type, addr);
            hw.arm9_write::<T>(addr, value);
        } else {
            self.cycle += hw.arm7_get_access_time::<T>(self.next_access_type, addr);
            hw.arm7_write::<T>(addr, value);
        }
        self.next_access_type = access_type;
    }

    pub fn instruction_prefetch<T: MemoryValue>(&mut self, hw: &mut HW, access_type: AccessType) {
        // Internal Cycle merges with instruction prefetch
        // TODO: Increment PC here
        self.instr_buffer[1] =
            num::cast::<T, u32>(self.read::<T>(hw, access_type, self.regs[15])).unwrap();
    }

    pub fn internal(&mut self) {
        self.cycle += 1;
        self.next_access_type = AccessType::N;
    }

    fn is_halted(&self, hw: &HW) -> bool {
        if IS_ARM9 { hw.cp15.arm9_halted } else { hw.haltcnt.halted() }
    }

    /// Handles a pending IRQ: unhalt the CPU, switch to IRQ mode, push LR,
    /// disable further IRQs (I-bit), and jump to the IRQ vector (base+18h;
    /// the ARM9 vector base is relocatable via CP15 to FFFF0000h).
    ///
    /// GBATEK "ARM CPU Exceptions":
    /// <https://problemkaputt.de/gbatek.htm#armcpuexceptions>
    pub fn handle_irq(&mut self, hw: &mut HW) {
        let (interrupts_requested, interrupt_base) = if IS_ARM9 {
            (hw.arm9_interrupts_requested(), hw.cp15.interrupt_base())
        } else {
            (hw.arm7_interrupts_requested(), 0)
        };
        let use_i = IS_ARM9 || !hw.haltcnt.halted();
        if likely((use_i && self.regs.get_i()) || !interrupts_requested) {
            return;
        }
        if IS_ARM9 {
            hw.cp15.arm9_halted = false
        } else {
            hw.haltcnt.unhalt();
        }
        self.regs.change_mode(Mode::IRQ);
        let lr = if unlikely(self.regs.get_t()) {
            self.read::<u16>(hw, AccessType::N, self.regs[15]);
            self.regs[15].wrapping_sub(2).wrapping_add(4)
        } else {
            self.read::<u32>(hw, AccessType::N, self.regs[15]);
            self.regs[15].wrapping_sub(4).wrapping_add(4)
        };
        self.regs.set_lr(lr);
        self.regs.set_t(false);
        self.regs.set_i(true);
        self.regs[15] = interrupt_base | 0x18;
        self.fill_arm_instr_buffer(hw);
    }

    pub(self) fn should_exec(&self, condition: u32) -> bool {
        self.condition_lut[((self.regs.get_flags() & 0xF0) | condition) as usize]
    }

    /// Performs a barrel-shifter operation and optionally updates CPSR flags.
    ///
    /// GBATEK "ARM Opcodes: Data Processing – barrel shifter":
    /// <https://problemkaputt.de/gbatek.htm#armopcodesdataprocessingalu>
    ///
    /// `shift_type`: 0=LSL, 1=LSR, 2=ASR, 3=ROR/RRX.
    /// `immediate`: when `true` the shift amount came from the instruction
    ///   encoding (special-case rules for shift==0 apply).
    pub(self) fn shift(
        &mut self,
        shift_type: u32,
        operand: u32,
        shift: u32,
        immediate: bool,
        change_status: bool,
    ) -> u32 {
        if immediate && shift == 0 {
            match shift_type {
                // LSL #0
                0 => operand,
                // LSR #32
                1 => {
                    if change_status {
                        self.regs.set_c(operand >> 31 != 0)
                    }
                    0
                }
                // ASR #32
                2 => {
                    let bit = operand >> 31 != 0;
                    if change_status {
                        self.regs.set_c(bit);
                    }
                    if bit { 0xFFFF_FFFF } else { 0 }
                }
                // RRX #1
                3 => {
                    let new_c = operand & 0x1 != 0;
                    let op2 = (self.regs.get_c() as u32) << 31 | operand >> 1;
                    if change_status {
                        self.regs.set_c(new_c)
                    }
                    op2
                }
                _ => unreachable!(),
            }
        } else if shift > 31 {
            assert!(!immediate);
            if !immediate {
                self.internal()
            }
            match shift_type {
                // LSL
                0 => {
                    if change_status {
                        if shift == 32 {
                            self.regs.set_c(operand << (shift - 1) & 0x8000_0000 != 0)
                        } else {
                            self.regs.set_c(false)
                        }
                    }
                    0
                }
                // LSR
                1 => {
                    if change_status {
                        if shift == 32 {
                            self.regs.set_c(operand >> (shift - 1) & 0x1 != 0)
                        } else {
                            self.regs.set_c(false)
                        }
                    }
                    0
                }
                // ASR
                2 => {
                    let c = operand & 0x8000_0000 != 0;
                    if change_status {
                        self.regs.set_c(c)
                    }
                    if c { 0xFFFF_FFFF } else { 0 }
                }
                // ROR
                3 => {
                    let shift = shift & 0x1F;
                    let shift = if shift == 0 { 0x20 } else { shift };
                    if change_status {
                        self.regs.set_c(operand >> (shift - 1) & 0x1 != 0)
                    }
                    operand.rotate_right(shift)
                }
                _ => unreachable!(),
            }
        } else {
            if !immediate {
                self.internal()
            }
            let change_status = change_status && shift != 0;
            match shift_type {
                // LSL
                0 => {
                    if change_status {
                        self.regs.set_c(operand << (shift - 1) & 0x8000_0000 != 0);
                    }
                    operand << shift
                }
                // LSR
                1 => {
                    if change_status {
                        self.regs.set_c(operand >> (shift - 1) & 0x1 != 0);
                    }
                    operand >> shift
                }
                // ASR
                2 => {
                    if change_status {
                        self.regs.set_c((operand as i32) >> (shift - 1) & 0x1 != 0)
                    };
                    ((operand as i32) >> shift) as u32
                }
                // ROR
                3 => {
                    if change_status {
                        self.regs.set_c(operand >> (shift - 1) & 0x1 != 0);
                    }
                    operand.rotate_right(shift)
                }
                _ => unreachable!(),
            }
        }
    }

    /// ADD: `op1 + op2`, optionally setting NZCV flags.
    pub(self) fn add(&mut self, op1: u32, op2: u32, change_status: bool) -> u32 {
        let result = op1.overflowing_add(op2);
        if change_status {
            self.regs.set_c(result.1);
            self.regs.set_v((op1 as i32).overflowing_add(op2 as i32).1);
            self.regs.set_z(result.0 == 0);
            self.regs.set_n(result.0 & 0x8000_0000 != 0);
        }
        result.0
    }

    /// ADC: `op1 + op2 + C`, optionally setting NZCV flags.
    pub(self) fn adc(&mut self, op1: u32, op2: u32, change_status: bool) -> u32 {
        let result = op1.overflowing_add(op2);
        let result2 = result.0.overflowing_add(self.regs.get_c() as u32);
        if change_status {
            self.regs.set_c(result.1 || result2.1);
            self.regs.set_z(result2.0 == 0);
            self.regs.set_n(result2.0 & 0x8000_0000 != 0);
            self.regs.set_v((!(op1 ^ op2)) & (op1 ^ result2.0) & 0x8000_0000 != 0);
        }
        result2.0
    }

    /// SUB: `op1 - op2` implemented as `op1 + NOT(op2) + 1` via [`adc`](Self::adc).
    pub(self) fn sub(&mut self, op1: u32, op2: u32, change_status: bool) -> u32 {
        let old_c = self.regs.get_c();
        self.regs.set_c(true);
        let result = self.adc(op1, !op2, change_status); // Simulate adding op1 + !op2 + 1
        if !change_status {
            self.regs.set_c(old_c)
        }
        result
    }

    /// SBC: `op1 - op2 - NOT(C)` = `op1 + NOT(op2) + C` via [`adc`](Self::adc).
    pub(self) fn sbc(&mut self, op1: u32, op2: u32, change_status: bool) -> u32 {
        self.adc(op1, !op2, change_status)
    }

    /// Adds extra internal cycles for a multiply instruction based on the number
    /// of non-trivial bytes in `op1` (ARM7TDMI / ARM9 multiplier early-out).
    pub(self) fn inc_mul_clocks(&mut self, op1: u32, signed: bool) {
        let mut mask = 0xFF_FF_FF_00;
        loop {
            self.internal();
            let value = op1 & mask;
            if mask == 0 || value == 0 || signed && value == mask {
                break;
            }
            mask <<= 8;
        }
    }
}

fn condition_lut() -> &'static [bool; 256] {
    static CONDITION_LUT: OnceLock<[bool; 256]> = OnceLock::new();
    CONDITION_LUT.get_or_init(instructions::gen_condition_table)
}

/// Returns the global ARM instruction LUT for the selected CPU type.
///
/// ARM7 and ARM9 use different instruction handler types because
/// `InstructionHandler` is parameterized by the `IS_ARM9` const generic.
///
/// Rust cannot currently unify:
///
///     InstructionHandler<_, true>
///     InstructionHandler<_, false>
///
/// even inside a compile-time `if IS_ARM9`.
///
/// Therefore we store separate static LUTs and cast them back to the
/// requested const-generic type.
///
/// # Safety
///
/// This cast is safe because:
///
/// - `IS_ARM9 == true` only accesses `ARM9_ARM_LUT`
/// - `IS_ARM9 == false` only accesses `ARM7_ARM_LUT`
///
/// so the runtime value and compile-time type always match.
fn arm_lut<const IS_ARM9: bool>() -> &'static ArmLut<IS_ARM9> {
    if IS_ARM9 {
        static ARM9_ARM_LUT: OnceLock<ArmLut<true>> = OnceLock::new();
        unsafe {
            &*(ARM9_ARM_LUT.get_or_init(arm::gen_lut) as *const ArmLut<true>
                as *const ArmLut<IS_ARM9>)
        }
    } else {
        static ARM7_ARM_LUT: OnceLock<ArmLut<false>> = OnceLock::new();
        unsafe {
            &*(ARM7_ARM_LUT.get_or_init(arm::gen_lut) as *const ArmLut<false>
                as *const ArmLut<IS_ARM9>)
        }
    }
}

/// Returns the global THUMB instruction LUT for the selected CPU type.
///
/// See `arm_lut()` for details about the const-generic cast.
fn thumb_lut<const IS_ARM9: bool>() -> &'static ThumbLut<IS_ARM9> {
    if IS_ARM9 {
        static ARM9_THUMB_LUT: OnceLock<ThumbLut<true>> = OnceLock::new();
        unsafe {
            &*(ARM9_THUMB_LUT.get_or_init(thumb::gen_lut) as *const ThumbLut<true>
                as *const ThumbLut<IS_ARM9>)
        }
    } else {
        static ARM7_THUMB_LUT: OnceLock<ThumbLut<false>> = OnceLock::new();
        unsafe {
            &*(ARM7_THUMB_LUT.get_or_init(thumb::gen_lut) as *const ThumbLut<false>
                as *const ThumbLut<IS_ARM9>)
        }
    }
}
