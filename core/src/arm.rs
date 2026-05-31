#[macro_use]
mod instructions;
mod arm;
mod registers;
mod thumb;

use crate::hw::{AccessType, HW, MemoryValue};
use crate::{likely, num, unlikely};
use registers::{Mode, RegValues};
use std::sync::OnceLock;

type ArmLut<const IS_ARM9: bool> = [instructions::InstructionHandler<u32, IS_ARM9>; 4096];
type ThumbLut<const IS_ARM9: bool> = [instructions::InstructionHandler<u16, IS_ARM9>; 256];

#[derive(emu_utils::Savestate)]
pub struct ARM<const IS_ARM9: bool> {
    cycle: usize,
    regs: RegValues,
    instr_buffer: [u32; 2],
    next_access_type: AccessType,

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

    pub(self) fn sub(&mut self, op1: u32, op2: u32, change_status: bool) -> u32 {
        let old_c = self.regs.get_c();
        self.regs.set_c(true);
        let result = self.adc(op1, !op2, change_status); // Simulate adding op1 + !op2 + 1
        if !change_status {
            self.regs.set_c(old_c)
        }
        result
    }

    pub(self) fn sbc(&mut self, op1: u32, op2: u32, change_status: bool) -> u32 {
        self.adc(op1, !op2, change_status)
    }

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
