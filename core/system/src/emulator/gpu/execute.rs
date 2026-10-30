// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! GX command execution: collects the parameters of the command at the
//! head of GXPIPE and runs it on the geometry engine.
use ds_gpu::engine_3d::gxfifo::{CMD_CYCLE_AMOUNTS, CMD_PARAM_AMOUNTS};

use crate::Emulator;

impl Emulator {
    /// Pops one GXPIPE entry and, once all parameters of its command are
    /// collected, runs the command on the geometry engine.
    ///
    /// ```text
    /// from: gpu3d_run() (normal path) / write_command() when GXFIFO is
    ///       full (the CPU stalls until a slot frees up)
    ///   |
    ///   v
    /// exec_command()
    ///   1. read_command()            pop GXPIPE, refill it from GXFIFO,
    ///                                re-check FIFO DMA / IRQ
    ///   2. store the parameter; return until CMD_PARAM_AMOUNTS are in
    ///   3. cycles += CMD_CYCLE_AMOUNTS[cmd]
    ///   4. engine_3d.execute_command(cmd)
    ///   5. cycles += geometry_pipeline_cycles()   vertex/lighting setup
    ///   |
    ///   v
    /// to:   back to gpu3d_run(), which waits until these cycles elapse
    /// ```
    pub fn exec_command(&mut self) {
        let Some(cmd) = self.read_command() else {
            return;
        };
        let e = &mut self.gpu.engine_3d;
        let index = cmd.command as usize;
        e.cmd_params[usize::from(e.cmd_param_count) & 31] = cmd.param;
        e.cmd_param_count += 1;
        if e.cmd_param_count < CMD_PARAM_AMOUNTS[index] {
            return;
        }

        #[cfg(feature = "log")]
        log::trace!(target: "gx", "cmd {index:02X} {:08X}", e.cmd_params[0]);
        e.cycles += i64::from(CMD_CYCLE_AMOUNTS[index]);
        let polygons_before = e.num_consecutive_polygons;
        e.execute_command(cmd.command);
        e.cmd_param_count = 0;
        let extra = self.geometry_pipeline_cycles(index, polygons_before);
        self.gpu.engine_3d.cycles += extra;
    }

    /// Extra geometry engine time not covered by the per-command table
    /// (called by exec_command() step 5):
    /// the vertex, normal (lighting) and polygon setup pipelines
    /// (melonDS `GPU3D::SubmitVertex`/`SubmitPolygon`/`CalculateLighting`).
    ///
    /// Without it the FIFO drained several times faster than on hardware.
    /// The SDK streams display lists by DMA from "FIFO less than half full"
    /// interrupts and expects the last chunk's DMA interrupt to arrive while
    /// the FIFO is still above half; draining too fast kept the GXFIFO
    /// interrupt asserted and starved the DMA interrupt (HeartGold intro).
    fn geometry_pipeline_cycles(&self, cmd: usize, polygons_before: u32) -> i64 {
        let e = &self.gpu.engine_3d;
        match cmd {
            // Vertex commands: vertex pipeline.
            0x23..=0x28 => {
                let mut cycles = 3 + 7;
                if e.num_consecutive_polygons != polygons_before {
                    // Polygon setup: quads take longer than triangles.
                    cycles += if e.polygon_mode & 1 != 0 { 35 } else { 26 };
                }
                cycles
            }
            // NORMAL: one cycle per enabled light (at least one) + pipeline.
            0x21 => i64::from((e.cur_polygon_attr & 0xF).count_ones().max(1)) + 7,
            _ => 0,
        }
    }
}
