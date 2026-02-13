use crate::cpu::arm_cpu::CpuType;
use crate::cpu::interpreter::arm_interpret;
use crate::cpu::interpreter::thumb_instruction::thumb_interpret;

use crate::emulator::Emulator;

impl Emulator {
    /// Run the emulator main loop.
    pub fn run(&mut self) {
        self.gpu.start_frame();

        while !self.gpu.is_frame_complete() {
            // Handle self.ARM9
            self.calculate_system_timestamp();
            while self.arm9.get_timestamp() < (self.system_timestamp << 1) {
                self.execute(CpuType::Arm9);
                self.run_timers9((self.arm9.cycles_ran() >> 1) as i32);
                self.gpu.run_3d((self.arm9.cycles_ran() >> 1) as u64);
            }

            // Now handle ARM7
            while self.arm7.get_timestamp() < self.system_timestamp {
                self.execute(CpuType::Arm7);
                self.run_timers7(self.arm7.cycles_ran() as i32);
            }

            if self.system_timestamp >= self.gpu_event.activation_time {
                self.gpu.handle_event(&self.gpu_event);
            }

            if self.system_timestamp >= self.dma_event.activation_time && self.dma_event.processing
            {
                self.dma_handle_event(); // DMA Method
            }

            self.cartridge_run(8);
        }

        if let Err(err) = self.cart.save_check() {
            #[cfg(feature = "tracing")]
            tracing::error!("{err}");
        };
    }

    pub fn execute(&mut self, cpu_type: CpuType) {
        let cpu_id = {
            // ARM7 or ARM9
            let arm = match cpu_type {
                CpuType::Arm7 => &mut self.arm7,
                CpuType::Arm9 => &mut self.arm9,
            };

            arm.last_timestamp = arm.timestamp;

            arm.cpu_id
        };

        let is_dma_active = self.dma_active();
        let timestamp = self.get_timestamp() << (1 - cpu_id);
        let is_interrupt = self.requesting_interrupt(cpu_id);

        {
            // ARM7 or ARM9
            let arm = self.get_cpu_mut(cpu_type);

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
        }

        // Fetch and execute instruction
        let thumb_on = self.get_cpu_mut(cpu_type).cpsr.thumb_on;
        let reg_15 = self.get_cpu(cpu_type).regs[15];

        if thumb_on {
            {
                let value = self.read_halfword(reg_15 - 2, cpu_type) as u32;

                let arm = self.get_cpu_mut(cpu_type);
                arm.current_instr = value;
                arm.add_s16_code(reg_15 - 2, 1);
                arm.regs[15] += 2;
            }
            thumb_interpret(self, cpu_type);
        } else {
            {
                let value = self.read_word(reg_15 - 4, cpu_type);

                let arm = self.get_cpu_mut(cpu_type);
                arm.current_instr = value;
                arm.add_s32_code(arm.regs[15] - 4, 1);
                arm.regs[15] += 4;
            }
            arm_interpret(self, cpu_type);
        }

        let irq_disabled = match cpu_type {
            CpuType::Arm7 => self.arm7.cpsr.irq_disabled,
            CpuType::Arm9 => self.arm9.cpsr.irq_disabled,
        };
        if is_interrupt && !irq_disabled {
            let arm = match cpu_type {
                CpuType::Arm7 => &mut self.arm7,
                CpuType::Arm9 => &mut self.arm9,
            };
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
    #[ignore = "Since need local nds file"]
    #[quick_tracing::init(test = "test_emulator", level = "DEBUG")]
    fn test_emulator() {
        // Run main emulator
        let mut emu = Box::new(Emulator::new());
        let rom_path = std::path::Path::new("../test_rom/test.nds");
        emu.load_rom(rom_path).unwrap();
        emu.run();
    }
}
