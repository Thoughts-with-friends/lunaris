//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp

use crate::Emulator;
use crate::interrupts::Interrupt;
use lunaris_ds_gpu::gpu_3d::consts::{CMD_PARAM_AMOUNTS, SCANLINES};
use lunaris_ds_gpu::gpu_3d::structs::GxCommand;

impl Emulator {
    /// - Instead of
    pub fn check_fifo_irq(&mut self) {
        match self.gpu.engine_3d.gxstat.gxfifo_irq_stat {
            1 if self.gpu.engine_3d.gxfifo.len() < 128 => {
                self.request_interrupt9(Interrupt::GeometryFifo)
            }
            2 if self.gpu.engine_3d.gxfifo.is_empty() => {
                self.request_interrupt9(Interrupt::GeometryFifo)
            }
            _ => {} //Never send IRQ requests
        }
    }

    pub fn check_gxfifo_irq(&mut self) {
        self.check_fifo_irq();
    }

    pub fn request_fifo_dma(&mut self) {
        if self.gpu.engine_3d.gxfifo.len() < 128 {
            self.gxfifo_dma_request();
        }
    }

    pub fn read_command(&mut self) -> Option<GxCommand> {
        let cmd = self.gpu.engine_3d.gxpipe.front().cloned();

        // Refill the pipe if it is at least half-empty
        let gxpipe_len = self.gpu.engine_3d.gxpipe.len();
        if gxpipe_len < 3 {
            if !self.gpu.engine_3d.gxfifo.is_empty()
                && let Some(front) = self.gpu.engine_3d.gxfifo.front()
            {
                let front = front.clone();
                self.gpu.engine_3d.gxpipe.push_back(front);
                self.gpu.engine_3d.gxfifo.pop_back();
            }
            if !self.gpu.engine_3d.gxfifo.is_empty()
                && let Some(front) = self.gpu.engine_3d.gxfifo.front()
            {
                let front = front.clone();
                self.gpu.engine_3d.gxpipe.push_back(front);
                self.gpu.engine_3d.gxfifo.pop_back();
            }

            self.check_fifo_dma();
            self.check_fifo_irq();
        }

        // Check if the next command is a BOX/POS/VEC test or matrix stack operation
        // And update relevant flags
        if !self.gpu.engine_3d.gxpipe.is_empty() {
            if let Some(next_cmd) = self.gpu.engine_3d.gxpipe.front() {
                self.gpu.engine_3d.gxstat.mtx_stack_busy =
                    next_cmd.command == 0x11 || next_cmd.command == 0x12;
                self.gpu.engine_3d.gxstat.box_pos_vec_busy = next_cmd.command == 0x70
                    || next_cmd.command == 0x71
                    || next_cmd.command == 0x72;
            };
        } else {
            self.gpu.engine_3d.gxstat.mtx_stack_busy = false;
            self.gpu.engine_3d.gxstat.box_pos_vec_busy = false;
        }

        cmd
    }

    fn write_command(&mut self, cmd: GxCommand) {
        #[cfg(feature = "tracing")]
        tracing::info!(?cmd.command, ?cmd.param);

        match self.gpu.engine_3d.gxfifo.is_empty() && self.gpu.engine_3d.gxpipe.len() < 4 {
            true => self.gpu.engine_3d.gxpipe.push_back(cmd),
            false => {
                if self.gpu.engine_3d.gxfifo.len() >= 256 {
                    while self.gpu.engine_3d.gxfifo.len() >= 256 {
                        self.exec_command();
                    }
                }
                self.gpu.engine_3d.gxfifo.push_back(cmd);
            }
        }
    }

    pub fn set_gxstat(&mut self, word: u32) {
        self.gpu.engine_3d.gxstat.gxfifo_irq_stat = ((word >> 30) & 0x3) as i32;
        self.check_fifo_irq();
    }

    pub fn write_gxfifo(&mut self, word: u32) {
        if self.gpu.engine_3d.cmd_count == 0 {
            self.gpu.engine_3d.cmd_count = 4;
            self.gpu.engine_3d.current_cmd = word;
            self.gpu.engine_3d.param_count = 0;
            self.gpu.engine_3d.total_params = CMD_PARAM_AMOUNTS[(word & 0xFF) as usize];

            if self.gpu.engine_3d.total_params > 0 {
                return;
            }
        } else {
            self.gpu.engine_3d.param_count += 1;
        }

        loop {
            if (self.gpu.engine_3d.current_cmd & 0xFF) != 0
                || (self.gpu.engine_3d.cmd_count == 4 && self.gpu.engine_3d.current_cmd == 0)
            {
                let cmd = GxCommand {
                    command: (self.gpu.engine_3d.current_cmd & 0xFF) as u8,
                    param: word,
                };
                self.write_command(cmd);
            }

            if self.gpu.engine_3d.param_count >= self.gpu.engine_3d.total_params {
                self.gpu.engine_3d.current_cmd >>= 8;
                self.gpu.engine_3d.cmd_count -= 1;

                if self.gpu.engine_3d.cmd_count == 0 {
                    break;
                }

                self.gpu.engine_3d.param_count = 0;
                self.gpu.engine_3d.total_params =
                    CMD_PARAM_AMOUNTS[(self.gpu.engine_3d.current_cmd & 0xFF) as usize];
            }

            if self.gpu.engine_3d.param_count < self.gpu.engine_3d.total_params {
                break;
            }
        }
    }

    pub fn write_fifo_direct(&mut self, address: u32, word: u32) {
        self.write_command(GxCommand {
            command: ((address >> 2) & 0x7F) as u8,
            param: word,
        });
    }

    pub fn check_fifo_dma(&mut self) {
        if self.gpu.engine_3d.gxfifo.len() < 128 {
            self.gxfifo_dma_request();
        }
    }

    pub fn gpu_handle_event(&mut self) {
        match self.gpu_event.id {
            0 => {
                // Start HBLANK
                if (self.gpu.vertical_count as usize) < SCANLINES
                    && self.gpu.frames_skipped >= self.config.frameskip
                {
                    self.gpu.draw_scanline();
                }

                #[cfg(feature = "tracing")]
                tracing::debug!("Start HBLANK");

                self.gpu.display_status_arm7.is_hblank = true;
                self.gpu.display_status_arm9.is_hblank = true;

                if self.gpu.display_status_arm7.irq_on_hblank {
                    self.request_interrupt7(Interrupt::HBlank);
                }
                if self.gpu.display_status_arm9.irq_on_hblank {
                    self.request_interrupt9(Interrupt::HBlank);
                }
                if (self.gpu.vertical_count as usize) < SCANLINES {
                    self.hblank_dma_request();
                }

                #[cfg(feature = "tracing")]
                tracing::debug!("Add_gpu_event: 1");

                self.add_gpu_event(1, 99 * 6);
            }
            1 => {
                // End HBLANK
                #[cfg(feature = "tracing")]
                tracing::debug!("End HBLANK");

                self.gpu.display_status_arm7.is_hblank = false;
                self.gpu.display_status_arm9.is_hblank = false;

                self.gpu.display_status_arm7.is_vcounter =
                    self.gpu.vertical_count == self.gpu.display_status_arm7.vcounter;
                if self.gpu.display_status_arm7.is_vcounter
                    && self.gpu.display_status_arm7.irq_on_vcounter
                {
                    self.request_interrupt7(Interrupt::VCountMatch);
                }

                self.gpu.display_status_arm9.is_vcounter =
                    self.gpu.vertical_count == self.gpu.display_status_arm9.vcounter;
                if self.gpu.display_status_arm9.is_vcounter
                    && self.gpu.display_status_arm9.irq_on_vcounter
                {
                    self.request_interrupt9(Interrupt::VCountMatch);
                }

                self.gpu.vertical_count += 1;
                // VBLANK Counter
                #[cfg(feature = "tracing")]
                tracing::debug!("VBLANK Count: {}/{}", self.gpu.vertical_count, SCANLINES);

                if self.gpu.vertical_count as usize == SCANLINES {
                    // VBLANK
                    #[cfg(feature = "tracing")]
                    tracing::debug!("Start VBLANK");

                    self.gpu.engine_3d.end_of_frame();
                    self.gpu.frame_complete = true;
                    if self.gpu.display_status_arm7.irq_on_vblank {
                        self.request_interrupt7(Interrupt::VBlank);
                    }
                    if self.gpu.display_status_arm9.irq_on_vblank {
                        self.request_interrupt9(Interrupt::VBlank);
                    }
                    self.gpu.display_status_arm7.is_vblank = true;
                    self.gpu.display_status_arm9.is_vblank = true;

                    self.gpu.engine_upper.vblank_start();
                    self.gpu.engine_lower.vblank_start();
                }
                if self.gpu.vertical_count == 263 {
                    self.gpu.vertical_count = 0;
                    self.gpu.display_status_arm7.is_vblank = false;
                    self.gpu.display_status_arm9.is_vblank = false;

                    match self.gpu.frames_skipped >= self.config.frameskip {
                        true => self.gpu.frames_skipped = 0,
                        false => self.gpu.frames_skipped += 1,
                    }
                }

                self.add_gpu_event(0, 256 * 6);
            }
            unknown_id => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized event.id: {unknown_id}");
            }
        }
    }
}
