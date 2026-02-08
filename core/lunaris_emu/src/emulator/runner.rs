use crate::cpu::interpreter::arm_interpret;
use crate::cpu::interpreter::thumb_instruction::thumb_interpret;

use crate::emulator::Emulator;

impl Emulator {
    /// Run the emulator main loop.
    pub fn run(&mut self) {
        const IS_ARM7: bool = true;
        self.gpu.start_frame();

        while !self.gpu.is_frame_complete() {
            // Handle self.ARM9
            self.calculate_system_timestamp();
            while self.arm9.get_timestamp() < (self.system_timestamp << 1) {
                self.execute(!IS_ARM7);
                self.run_timers9((self.arm9.cycles_ran() >> 1) as i32);
                self.gpu.run_3D((self.arm9.cycles_ran() >> 1) as u64);
            }

            // Now handle ARM7
            while self.arm7.get_timestamp() < self.system_timestamp {
                self.execute(IS_ARM7);
                self.run_timers7(self.arm7.cycles_ran() as i32);
            }

            if self.system_timestamp >= self.gpu_event.activation_time {
                self.gpu.handle_event(&self.gpu_event);
            }

            if self.system_timestamp >= self.dma_event.activation_time && self.dma_event.processing
            {
                self.dma.handle_event(&self.dma_event);
            }

            self.cart.run(8);
        }
        self.cart.save_check();
    }

    pub fn execute(&mut self, is_arm7: bool) {
        let cpu_id = {
            // ARM7 or ARM9
            let arm = match is_arm7 {
                true => &mut self.arm7,
                false => &mut self.arm9,
            };

            arm.last_timestamp = arm.timestamp;

            arm.cpu_id
        };

        let is_dma_active = self.dma_active();
        let timestamp = self.get_timestamp() << (1 - cpu_id);
        let is_interrupt = self.requesting_interrupt(cpu_id);

        // ARM7 or ARM9
        let arm = match is_arm7 {
            true => &mut self.arm7,
            false => &mut self.arm9,
        };

        if arm.halted || is_dma_active {
            // Wait until next event
            arm.timestamp = timestamp;
            if is_interrupt {
                arm.halted = false;
                if !arm.cpsr.irq_disabled && !is_dma_active {
                    arm.handle_irq();
                }
            }
            return;
        }

        // Fetch and execute instruction
        if arm.cpsr.thumb_on {
            arm.current_instr = arm.read_halfword(arm.regs[15] - 2) as u32;
            arm.add_s16_code(arm.regs[15] - 2, 1);
            arm.regs[15] += 2;
            thumb_interpret(arm);
        } else {
            arm.current_instr = arm.read_word(arm.regs[15] - 4);
            arm.add_s32_code(arm.regs[15] - 4, 1);
            arm.regs[15] += 4;
            arm_interpret(arm);
        }

        // interpreter(&mut Emu);
        //   -> arm.read_dram(&mut Emu.dram)
        //
        // interpreter(&mut Emu)
        //
        // impl Engine for Emu {
        //    fn get_cpu_mut(&mut self) -> &mut ArmCpu;
        //    fn read_halfword(&mut self) -> u32;
        // }
        // interpreter(emu: &mut impl Engine)
        //    -> emu.get_halfword(&mut self) -> u32
        //
        // arm -> Emu -> interpreter

        if is_interrupt && !arm.cpsr.irq_disabled {
            arm.handle_irq();
        }
    }
}

#[cfg(test)]
mod tests {
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
