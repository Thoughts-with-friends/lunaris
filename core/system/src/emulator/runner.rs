use ds_cpu::{ARM_INST_WIDTH, THUMB_INST_WIDTH, arm_cpu::CpuType};

use crate::{
    cpu::interpreter::{arm_interpret, thumb_instruction::thumb_interpret},
    emulator::Emulator,
};

impl Emulator {
    /// Runs the emulator for one video frame (263 scanlines, 560190 bus
    /// cycles). Called once per host frame by the frontends.
    ///
    /// The two CPUs and the devices are interleaved in small time slices:
    /// the scheduler picks the end of the next slice, the ARM9 (66 MHz) and
    /// then the ARM7 (33 MHz) execute until they reach it, and the devices
    /// that came due fire.
    ///
    /// ```text
    /// from: gui/egui main.rs render_frame / gui/tauri emu_thread::run /
    ///       examples headless main            (input already applied)
    ///   |
    ///   v
    /// run()
    ///   1. gpu.start_frame()                  clear the frame-complete flag
    ///      wifi.new_frame()                   let the MP link pump its
    ///                                         transport (ds_net)
    ///      run_cheats()                       Action Replay codes (cheats.rs)
    ///   2. loop until gpu.is_frame_complete() (set when line 192 starts
    ///                                         VBlank, gpu_handle_event)
    ///      a. calculate_system_timestamp()    system_timestamp += slice
    ///                                         (<= 64 cycles or next event)
    ///      b. ARM9 catch-up, while arm9 time < 2 * system_timestamp:
    ///           execute(Arm9) -> run_timers9(ran) -> run_3d(ran)
    ///      c. ARM7 catch-up, while arm7 time < system_timestamp:
    ///           execute(Arm7) -> run_timers7(ran) -> run_spu(ran)
    ///      d. wifi.run(slice) -> raise_wifi_irq()
    ///      e. gpu_event due?  -> gpu_handle_event()   HBlank/VBlank/lines
    ///      f. dma_event due?  -> dma_handle_event()   pending DMA transfers
    ///      g. cartridge_run(slice)                    ROM data transfer
    ///   3. cart.save_check()                  flush the .sav when idle
    ///   |
    ///   v
    /// to:   caller reads get_upper_frame / get_lower_frame and
    ///       take_audio_samples, then calls run() again next frame
    /// ```
    pub fn run(&mut self) {
        self.gpu.start_frame();
        self.wifi.new_frame();
        self.run_cheats();

        while !self.gpu.is_frame_complete() {
            // Handle self.ARM9
            let before = self.system_timestamp;
            self.calculate_system_timestamp();
            let slice = (self.system_timestamp - before) as i32;
            while self.arm9.get_timestamp() < (self.system_timestamp << 1) {
                self.execute(CpuType::Arm9);
                // ARM9 timestamps count 66 MHz cycles. Halve the timestamps
                // rather than the difference: `cycles_ran() >> 1` truncates
                // one-cycle steps to 0, so a tight GXSTAT/timer polling
                // loop never let the 3D engine or the timers advance
                // (Pokemon Ranger waited forever on GXSTAT busy).
                let ran =
                    (self.arm9.timestamp >> 1).wrapping_sub(self.arm9.last_timestamp >> 1) as i64;
                self.run_timers9(ran as i32);
                self.run_3d(ran);
            }

            // Now handle ARM7
            while self.arm7.get_timestamp() < self.system_timestamp {
                self.execute(CpuType::Arm7);
                self.run_timers7(self.arm7.cycles_ran() as i32);
                self.run_spu(self.arm7.cycles_ran() as u32);
            }
            self.wifi.run(slice as u32);
            self.raise_wifi_irq();

            if self.system_timestamp >= self.gpu_event.activation_time {
                self.gpu_handle_event();
            }

            if self.system_timestamp >= self.dma_event.activation_time && self.dma_event.processing
            {
                self.dma_handle_event(); // DMA Method
            }

            self.cartridge_run(slice);
        }

        if let Err(_err) = self.cart.save_check() {
            #[cfg(feature = "log")]
            log::error!("{_err}");
        };
    }

    /// Executes one instruction on `cpu_type`, or idles it when it is
    /// halted or the bus is held by a DMA.
    ///
    /// ```text
    /// from: run() step 2b (ARM9) / 2c (ARM7)
    ///   |
    ///   v
    /// execute(cpu)
    ///   1. last_timestamp = timestamp          (cycles_ran() is measured
    ///                                           from here)
    ///   2. halted or DMA active?
    ///        yes: timestamp = system time      (jump to the slice end)
    ///             IE & IF != 0 -> leave HALT
    ///             IME && !CPSR.I -> handle_irq()
    ///             return
    ///   3. trace hooks (Config::trace_trigger / trace_cpus)
    ///   4. fetch at pc-2 (Thumb) or pc-4 (ARM) via read_halfword/read_word,
    ///      record_pc, add code cycles, advance r15
    ///   5. thumb_interpret() / arm_interpret()  decode + run the handler
    ///      (memory accesses go through read_* / write_* -> devices)
    ///   6. IRQ pending and CPSR.I clear -> handle_irq()
    ///   |
    ///   v
    /// to:   back to run(), which advances timers / 3D / SPU by cycles_ran()
    /// ```
    pub fn execute(&mut self, cpu_type: CpuType) {
        let cpu_id = {
            // ARM7 or ARM9
            let arm = self.get_cpu_mut(cpu_type);
            arm.last_timestamp = arm.timestamp;
            arm.cpu_id
        };

        {
            // ARM7 or ARM9
            let halted = self.get_cpu_mut(cpu_type).halted;
            let is_dma_active = self.dma_active();

            // #[cfg(feature = "log")]
            // log::info!(%halted, %is_dma_active);

            if halted || is_dma_active {
                let timestamp = self.get_timestamp() << (1 - cpu_id);

                // Wait until next event. HALT is released by any enabled
                // and requested IRQ (IE & IF) even while IME is 0 (GBATEK
                // "DS Interrupts"); the exception itself is only taken when
                // IME is set and CPSR.I is clear.
                let wake = self.irq_line_raised(cpu_id);
                let take_irq = self.requesting_interrupt(cpu_id);
                let arm = self.get_cpu_mut(cpu_type);
                arm.timestamp = timestamp;

                if wake && halted {
                    arm.halted = false;
                }
                if take_irq && !arm.halted && !arm.cpsr.irq_disabled && !is_dma_active {
                    arm.handle_irq();
                }
                return;
            }
        }

        // Fetch and execute thumb instruction
        let thumb_on = self.get_cpu_mut(cpu_type).cpsr.thumb_on;
        let pc = self.get_cpu(cpu_type).get_pc();

        if let Some((mask, at)) = self.config.trace_trigger
            && mask & (1 << cpu_id) != 0
            && pc.wrapping_sub(if thumb_on { THUMB_INST_WIDTH } else { ARM_INST_WIDTH }) == at
        {
            self.config.trace_cpus = mask;
            self.config.trace_trigger = None;
        }
        if self.config.trace_cpus & (1 << cpu_id) != 0 {
            self.trace_instruction(cpu_type, thumb_on, pc);
        }

        if thumb_on {
            {
                let value = self.read_halfword(pc - THUMB_INST_WIDTH, cpu_type) as u32;
                let arm = self.get_cpu_mut(cpu_type);

                arm.current_instr = value;
                arm.record_pc(pc - THUMB_INST_WIDTH);
                arm.add_s16_code(pc - THUMB_INST_WIDTH, 1);
                arm.regs[15] = pc.wrapping_add(THUMB_INST_WIDTH);
            }
            thumb_interpret(self, cpu_type);
        } else {
            {
                let addr = pc.wrapping_sub(ARM_INST_WIDTH);
                let value = self.read_word(addr, cpu_type);
                let arm = self.get_cpu_mut(cpu_type);

                arm.current_instr = value;
                arm.record_pc(addr);
                arm.add_s32_code(addr, 1);
                arm.regs[15] = pc.wrapping_add(ARM_INST_WIDTH);
            }
            arm_interpret(self, cpu_type);
        }

        let is_interrupt = self.requesting_interrupt(cpu_id);
        let irq_disabled = self.get_cpu(cpu_type).cpsr.irq_disabled;

        if is_interrupt && !irq_disabled {
            self.get_cpu_mut(cpu_type).handle_irq();
        }
    }

    /// Prints the instruction about to execute (see `Config::trace_cpus`).
    ///
    /// ```text
    /// from: execute() step 3, only while tracing is enabled for this CPU
    /// to:   stderr line "<9|7><A|T> addr opcode cpsr r0..r14"
    /// ```
    #[cold]
    fn trace_instruction(&mut self, cpu_type: CpuType, thumb_on: bool, pc: u32) {
        let addr = pc.wrapping_sub(if thumb_on { 2 } else { 4 });
        let op = if thumb_on {
            u32::from(self.read_halfword(addr, cpu_type))
        } else {
            self.read_word(addr, cpu_type)
        };
        let cpu = self.get_cpu(cpu_type);
        let r = &cpu.regs;
        eprintln!(
            "{}{} {addr:08X} {op:08X} cpsr={:08X} r={:08X?}",
            if cpu.cpu_id == 0 { 9 } else { 7 },
            if thumb_on { 'T' } else { 'A' },
            cpu.cpsr.get(),
            &r[..15],
        );
    }
}
