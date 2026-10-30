//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!
use crate::Emulator;

impl Emulator {
    /// Advances the 3D geometry engine by `cycles` bus cycles.
    ///
    /// ```text
    /// from: run() step 2b, after every ARM9 instruction
    ///   |
    ///   v
    /// run_3d(cycles) -> gpu3d_run(cycles)
    /// ```
    pub fn run_3d(&mut self, cycles: i64) {
        self.gpu3d_run(cycles);
    }

    /// Spends `cycles_to_run` cycles of geometry engine time, executing GX
    /// commands from GXPIPE whenever the previous one has finished.
    ///
    /// ```text
    /// from: run_3d()
    ///   |
    ///   v
    /// gpu3d_run(cycles)
    ///   1. SWAP_BUFFERS pending?  only let time pass, run nothing
    ///                             (commands resume after VBlank)
    ///   2. idle (no time owed, GXPIPE empty)?  return
    ///   3. engine cycles -= cycles
    ///   4. while the engine is free and GXPIPE has commands:
    ///        exec_command()       (adds the command's cycle cost)
    ///   |
    ///   v
    /// to:   exec_command() -> engine_3d.execute_command() (matrices,
    ///       vertices, polygons); rendering itself happens at line 215
    /// ```
    pub fn gpu3d_run(&mut self, cycles_to_run: i64) {
        if self.gpu.engine_3d.swap_buffers {
            // Commands wait for VBlank, but the time of the ones already
            // executed (SWAP_BUFFERS itself included) still elapses.
            // Freezing it kept GXSTAT busy across every frame for games
            // that swap and then poll busy (Pokemon Ranger).
            let e = &mut self.gpu.engine_3d;
            e.cycles = (e.cycles - cycles_to_run).max(0);
            return;
        }
        if self.gpu.engine_3d.cycles <= 0 && self.gpu.engine_3d.gxpipe.is_empty() {
            self.gpu.engine_3d.cycles = 0;
            return;
        }

        self.gpu.engine_3d.cycles -= cycles_to_run;
        while self.gpu.engine_3d.cycles <= 0 && !self.gpu.engine_3d.gxpipe.is_empty() {
            self.exec_command();
        }
    }
}
