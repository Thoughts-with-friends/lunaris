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
                self.run_3d(self.arm9.cycles_ran() >> 1);
            }

            // Now handle ARM7
            while self.arm7.get_timestamp() < self.system_timestamp {
                self.execute(CpuType::Arm7);
                self.run_timers7(self.arm7.cycles_ran() as i32);
            }

            if self.system_timestamp >= self.gpu_event.activation_time {
                self.gpu_handle_event();
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

        {
            // ARM7 or ARM9
            let halted = self.get_cpu_mut(cpu_type).halted;
            let is_dma_active = self.dma_active();

            // #[cfg(feature = "tracing")]
            // tracing::info!(%halted, %is_dma_active);

            if halted || is_dma_active {
                let timestamp = self.get_timestamp() << (1 - cpu_id);

                // Wait until next event
                let is_interrupt = self.requesting_interrupt(cpu_id);
                let arm = self.get_cpu_mut(cpu_type);
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
        let pc = self.get_cpu(cpu_type).get_pc();

        if thumb_on {
            {
                let value = self.read_halfword(pc - 2, cpu_type) as u32;
                let arm = self.get_cpu_mut(cpu_type);

                arm.current_instr = value;
                arm.add_s16_code(pc - 2, 1);
                arm.regs[15] = pc.wrapping_add(2);
            }
            thumb_interpret(self, cpu_type);
        } else {
            {
                let addr = pc.wrapping_sub(4);
                let value = self.read_word(addr, cpu_type);
                let arm = self.get_cpu_mut(cpu_type);

                arm.current_instr = value;
                arm.add_s32_code(addr, 1);
                arm.regs[15] = pc.wrapping_add(4);
            }
            arm_interpret(self, cpu_type);
        }

        let is_interrupt = self.requesting_interrupt(cpu_id);
        let irq_disabled = self.get_cpu(cpu_type).cpsr.irq_disabled;

        if is_interrupt && !irq_disabled {
            self.get_cpu_mut(cpu_type).handle_irq();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Config;

    use super::*;
    use lunaris_ds_mem_const::{PIXELS_PER_LINE, SCANLINES};

    #[test]
    #[ignore = "Since need local nds file"]
    #[quick_tracing::init(test = "test_emulator", level = "trace")]
    fn test_emulator() {
        // Run main emulator
        let mut emu = Box::new(Emulator::new());
        let rom_path = std::path::Path::new("../test_rom/hello_world.nds");
        // let rom_path = std::path::Path::new("../test_rom/test.nds");

        emu.load_rom(rom_path).unwrap();

        const PIXEL: usize = PIXELS_PER_LINE * SCANLINES;
        let mut upper_buffer: [u32; PIXEL] = [0; PIXEL];
        let mut lower_buffer: [u32; PIXEL] = [0; PIXEL];

        let mut frames: u64 = 0;
        loop {
            #[cfg(feature = "tracing")]
            tracing::info!("frame: {frames}");

            // let last_update = std::time::Instant::now();
            emu.run();

            frames += 1;
            emu.get_upper_frame(&mut upper_buffer); // RGBA
            emu.get_lower_frame(&mut lower_buffer); // RGBA

            // upper display
            save_frame_as_png(
                &upper_buffer,
                PIXELS_PER_LINE as u32,
                SCANLINES as u32,
                format!("frame_upper_{:05}.png", frames),
            )
            .unwrap();

            // Lower display
            save_frame_as_png(
                &lower_buffer,
                PIXELS_PER_LINE as u32,
                SCANLINES as u32,
                format!("frame_lower_{:05}.png", frames),
            )
            .unwrap();
        }
    }

    pub fn save_frame_as_png<P>(
        buffer: &[u32],
        width: u32,
        height: u32,
        path: P,
    ) -> Result<(), Box<dyn core::error::Error + Send + Sync + 'static>>
    where
        P: AsRef<std::path::Path>,
    {
        use image::{ImageBuffer, Rgba};

        // 1D u32 -> u8: le RGBA [0xAARRGGBB]
        let raw_bytes: Vec<u8> = buffer.iter().flat_map(|px| px.to_le_bytes()).collect();

        // ImageBuffer
        let img: ImageBuffer<Rgba<u8>, _> = ImageBuffer::from_raw(width, height, raw_bytes)
            .ok_or("Failed to create image buffer")?;

        // Save PNG
        img.save(path)?;

        Ok(())
    }

    #[test]
    #[ignore = "Since need local nds file"]
    #[quick_tracing::init(test = "test_emulator", level = "trace")]
    fn test_emulator_mp4() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut emu = Box::new(Emulator {
            config: Config {
                test: true,
                ..Default::default()
            },
            ..Default::default()
        });
        // let rom_path = std::path::Path::new("../test_rom/hello_world.nds");
        let rom_path = std::path::Path::new("../test_rom/test.nds");
        emu.load_rom(rom_path).unwrap();

        const PIXEL: usize = PIXELS_PER_LINE * SCANLINES;
        let mut upper_buffer: [u32; PIXEL] = [0; PIXEL];
        let mut lower_buffer: [u32; PIXEL] = [0; PIXEL];

        const FPS: u32 = 60;
        let width = PIXELS_PER_LINE as u32;
        let height = SCANLINES as u32;
        let combined_width = width * 2;
        let end_frames = 600; // 10[s]

        // FFmpeg process for side-by-side video
        let mut ffmpeg = Command::new("ffmpeg")
            .args([
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-s",
                &format!("{}x{}", combined_width, height),
                "-r",
                &FPS.to_string(),
                "-i",
                "-",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "../test_rom/display.mp4",
            ])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();

        let ffmpeg_stdin = ffmpeg.stdin.as_mut().unwrap();

        let mut frames: u64 = 0;
        let mut combined_buffer: Vec<u32> = vec![0; (PIXELS_PER_LINE * 2) * SCANLINES];

        loop {
            emu.run();
            frames += 1;

            emu.get_upper_frame(&mut upper_buffer);
            emu.get_lower_frame(&mut lower_buffer);

            for y in 0..SCANLINES {
                let line_start = y * PIXELS_PER_LINE;
                let combined_line_start = y * (PIXELS_PER_LINE * 2);

                // lower display
                combined_buffer[combined_line_start..combined_line_start + PIXELS_PER_LINE]
                    .copy_from_slice(&lower_buffer[line_start..line_start + PIXELS_PER_LINE]);

                // upper display
                combined_buffer[combined_line_start + PIXELS_PER_LINE
                    ..combined_line_start + PIXELS_PER_LINE * 2]
                    .copy_from_slice(&upper_buffer[line_start..line_start + PIXELS_PER_LINE]);
            }

            // RGBA u32 -> u8
            let combined_bytes: Vec<u8> = combined_buffer
                .iter()
                .flat_map(|px| px.to_le_bytes())
                .collect();

            // Write FFmpeg
            if let Err(e) = ffmpeg_stdin.write_all(&combined_bytes) {
                eprintln!("FFmpeg pipe broken: {e}");
                break;
            }

            if frames >= end_frames {
                break;
            }
        }

        ffmpeg.wait().unwrap();
    }
}
