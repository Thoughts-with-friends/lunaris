//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!
use crate::Emulator;
use crate::interrupts::Interrupt;
use lunaris_ds_gpu::gpu_3d::structs::GxCommand;

impl Emulator {
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

            self.gpu.engine_3d.check_fifo_dma();
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
}
