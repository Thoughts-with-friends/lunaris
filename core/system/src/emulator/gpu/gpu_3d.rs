//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp

use ds_gpu::engine_3d::{gxfifo::CMD_PARAM_AMOUNTS, state::GxCommand};
use ds_interrupt::Interrupt;
use ds_mem_addr::SCANLINES;

use crate::Emulator;

impl Emulator {
    /// Re-evaluates the GXFIFO interrupt (GXSTAT bits 30-31).
    ///
    /// ```text
    /// from: read_command() / write_command() / set_gxstat()
    ///       (every time the FIFO level or the IRQ mode changes)
    ///   |
    ///   v
    /// check_fifo_irq()
    ///   mode 1: raise while len < 128 (less than half full)
    ///   mode 2: raise while len == 0  (empty)
    ///   otherwise / condition false: clear IF9 bit 21
    ///   |
    ///   v
    /// to:   execute() takes the IRQ on the ARM9 when IE/IME allow it
    /// ```
    ///
    /// The interrupt is level triggered (melonDS `GPU3D::CheckFIFOIRQ`):
    /// IF bit 21 follows the condition and is also *cleared* once the FIFO
    /// refills. It must be re-checked whenever the FIFO level changes.
    ///
    /// The Nitro SDK streams display lists with "FIFO less than half full"
    /// interrupts and switches them off from the DMA-complete interrupt of
    /// the last chunk. Leaving bit 21 set after the DMA refilled the FIFO
    /// made it win the IRQ priority over the DMA interrupt forever (an
    /// interrupt storm), which froze 3D scenes such as the HeartGold intro.
    pub fn check_fifo_irq(&mut self) {
        let len = self.gpu.engine_3d.gxfifo.len();
        let raised = match self.gpu.engine_3d.gxfifo_irq_mode() {
            1 => len < 128,
            2 => len == 0,
            _ => false,
        };
        let bit = 1 << (Interrupt::GeometryFifo as u32);
        if raised {
            self.int9_reg.irq_flags |= bit;
        } else {
            self.int9_reg.irq_flags &= !bit;
        }
    }

    /// Alias of [`Self::check_fifo_irq`] (CorgiDS naming).
    pub fn check_gxfifo_irq(&mut self) {
        self.check_fifo_irq();
    }

    /// Starts GXFIFO-mode DMAs while the FIFO is less than half full.
    pub fn request_fifo_dma(&mut self) {
        if self.gpu.engine_3d.gxfifo.len() < 128 {
            self.gxfifo_dma_request();
        }
    }

    /// Dequeues the oldest command/parameter word for execution.
    ///
    /// ```text
    /// from: exec_command() step 1
    ///   |
    ///   v
    /// read_command()
    ///   1. pop the front of GXPIPE (4-entry pipe in front of GXFIFO)
    ///   2. GXPIPE < 3 entries? move up to 2 words GXFIFO -> GXPIPE,
    ///      then check_fifo_dma() and check_fifo_irq()
    ///   |
    ///   v
    /// to:   exec_command() with the popped word (None = pipe was empty)
    /// ```
    pub fn read_command(&mut self) -> Option<GxCommand> {
        // GXPIPE/GXFIFO are FIFOs (matching C++ `std::queue`), so the command
        // that has been waiting longest must be dequeued from the front.
        // Reading via `front().cloned()` without removing it left the same
        // command in place forever, spinning `gpu3d_run` in an infinite loop
        // on the first GX command sent by a 3D game.
        let cmd = self.gpu.engine_3d.gxpipe.pop_front();

        // Refill the pipe if it is at least half-empty
        let gxpipe_len = self.gpu.engine_3d.gxpipe.len();
        if gxpipe_len < 3 {
            for _ in 0..2 {
                if let Some(front) = self.gpu.engine_3d.gxfifo.pop_front() {
                    self.gpu.engine_3d.gxpipe.push_back(front);
                }
            }

            self.check_fifo_dma();
            self.check_fifo_irq();
        }

        cmd
    }

    /// Queues one command/parameter word written by the ARM9 or a DMA.
    ///
    /// ```text
    /// from: write_gxfifo() (packed FIFO 0x04000400) /
    ///       write_fifo_direct() (command ports 0x04000440..)
    ///   |
    ///   v
    /// write_command(cmd)
    ///   GXFIFO empty and GXPIPE < 4  -> push straight into GXPIPE
    ///   otherwise:
    ///     GXFIFO full (256)?  exec_command() until a slot is free
    ///     push into GXFIFO, check_fifo_irq()
    ///   |
    ///   v
    /// to:   gpu3d_run() consumes it later
    /// ```
    fn write_command(&mut self, cmd: GxCommand) {
        #[cfg(feature = "log")]
        log::trace!("command={:?}, param={:?}", cmd.command, cmd.param);

        if self.gpu.engine_3d.gxfifo.is_empty() && self.gpu.engine_3d.gxpipe.len() < 4 {
            self.gpu.engine_3d.gxpipe.push_back(cmd);
        } else {
            if self.gpu.engine_3d.gxfifo.len() >= 256 {
                while self.gpu.engine_3d.gxfifo.len() >= 256 {
                    self.exec_command();
                }
            }
            self.gpu.engine_3d.gxfifo.push_back(cmd);
            self.check_fifo_irq();
        }
    }

    /// GXSTAT write (`mask` selects the written bytes), from the ARM9 I/O
    /// write handlers; re-checks the FIFO IRQ because its mode may change.
    pub fn set_gxstat(&mut self, word: u32, mask: u32) {
        self.gpu.engine_3d.write_gxstat(word, mask);
        self.check_fifo_irq();
    }

    /// Unpacks a write to GXFIFO (0x04000400): up to four packed command
    /// bytes followed by their parameters.
    ///
    /// ```text
    /// from: arm9_write_word(0x04000400..0x0400043F) / GXFIFO-mode DMA
    ///   |
    ///   v
    /// write_gxfifo(word)
    ///   1. no packed command in progress: latch `word` as up to 4
    ///      command bytes; wait for parameters if the first needs any
    ///   2. otherwise count `word` as the next parameter
    ///   3. loop: emit (command, word) via write_command(); when the
    ///      current command has all its parameters, shift to the next
    ///      packed command byte (commands without parameters are emitted
    ///      back to back)
    ///   |
    ///   v
    /// to:   write_command()
    /// ```
    pub fn write_gxfifo(&mut self, word: u32) {
        if self.gpu.engine_3d.cmd_count == 0 {
            self.gpu.engine_3d.cmd_count = 4;
            self.gpu.engine_3d.current_cmd = word;
            self.gpu.engine_3d.param_count = 0;
            // DS 3D Geometry Commands: https://problemkaputt.de/gbatek.htm#ds3dgeometrycommands
            self.gpu.engine_3d.total_params = CMD_PARAM_AMOUNTS[(word & 0xFF) as usize]; // command ID is the low byte of the packed GXFIFO word

            if self.gpu.engine_3d.total_params > 0 {
                return;
            }
        } else {
            self.gpu.engine_3d.param_count += 1;
        }

        loop {
            if (self.gpu.engine_3d.current_cmd & 0xFF) != 0 // non-zero command still waiting for parameters
                || (self.gpu.engine_3d.cmd_count == 4 && self.gpu.engine_3d.current_cmd == 0)
            {
                let cmd = GxCommand {
                    command: (self.gpu.engine_3d.current_cmd & 0xFF) as u8, // command ID (bits 0-7)
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
                    CMD_PARAM_AMOUNTS[(self.gpu.engine_3d.current_cmd & 0xFF) as usize]; // command ID (bits 0-7)
            }

            if self.gpu.engine_3d.param_count < self.gpu.engine_3d.total_params {
                break;
            }
        }
    }

    /// Command port write (0x04000440..0x040005CB): the address selects the
    /// command, `word` is its parameter. Goes to write_command().
    pub fn write_fifo_direct(&mut self, address: u32, word: u32) {
        self.write_command(GxCommand {
            command: ((address >> 2) & 0x7F) as u8, // command port 04000440h+4*n maps to command ID n (bits 2-8 of the address)
            param: word,
        });
    }

    /// Starts GXFIFO-mode DMAs while the FIFO is less than half full
    /// (called from read_command() after GXPIPE was refilled).
    pub fn check_fifo_dma(&mut self) {
        if self.gpu.engine_3d.gxfifo.len() < 128 {
            self.gxfifo_dma_request();
        }
    }

    /// Handles the scanline event scheduled by the GPU (two per line).
    ///
    /// One line = 355 dots * 6 cycles: 256 dots of drawing, then 99 dots
    /// of HBlank. Lines 192..262 are VBlank.
    ///
    /// ```text
    /// from: run() step 2e, when system time reaches gpu_event
    ///   |
    ///   v
    /// gpu_handle_event()
    ///   id 0 = start of HBlank (dot 256)
    ///     1. visible line and not frame-skipped -> gpu.draw_scanline()
    ///     2. DISPSTAT HBlank flag on; HBlank IRQ (ARM7/ARM9) if enabled
    ///     3. visible line -> hblank_dma_request()
    ///     4. reschedule id 1 in 99 * 6 cycles
    ///   id 1 = end of HBlank (next line)
    ///     1. HBlank flag off; V-count match flag + IRQ per CPU
    ///     2. vertical_count += 1
    ///     3. line 215: gpu.render_3d_frame()    (shown next frame)
    ///     4. line 192: VBlank start -> engine_3d.end_of_frame() (swap),
    ///        frame_complete = true, VBlank IRQs, 2D vblank_start()
    ///     5. line 263: wrap to 0, VBlank flag off, frameskip counter
    ///     6. reschedule id 0 in 256 * 6 cycles
    ///   |
    ///   v
    /// to:   run() ends the frame once frame_complete is set
    /// ```
    pub fn gpu_handle_event(&mut self) {
        match self.gpu_event.id {
            0 => {
                // Start HBLANK
                #[cfg(feature = "log")]
                log::trace!("Start HBLANK");

                // If the current line is within the visible screen
                // and the frameskip condition is met, the GPU draws this line.
                if (self.gpu.vertical_count as usize) < SCANLINES
                    && self.gpu.frames_skipped >= self.config.frameskip
                {
                    self.gpu.draw_scanline();
                }

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

                #[cfg(feature = "log")]
                log::trace!("Add_gpu_event: 1");

                self.reschedule_gpu_event(1, 99 * 6);
            }
            1 => {
                // End HBLANK
                #[cfg(feature = "log")]
                log::trace!("End HBLANK");

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
                #[cfg(feature = "log")]
                log::trace!("VBLANK Count: {}/{}", self.gpu.vertical_count, SCANLINES);

                // 3D rendering starts at line 215 (melonDS), after the
                // VBlank texture uploads, and is shown in the next frame.
                if self.gpu.vertical_count == 215
                    && self.gpu.frames_skipped >= self.config.frameskip
                {
                    self.gpu.render_3d_frame();
                }

                if self.gpu.vertical_count as usize == SCANLINES {
                    // VBLANK
                    #[cfg(feature = "log")]
                    log::trace!("Start VBLANK");

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

                self.reschedule_gpu_event(0, 256 * 6);
            }
            _unknown_id => {
                #[cfg(feature = "log")]
                log::error!("Unrecognized event.id: {_unknown_id}");
            }
        }
    }
}
