//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!
use crate::Emulator;

impl Emulator {
    /// Run 3D rendering for specified cycles
    pub fn run_3d(&mut self, cycles: i64) {
        self.gpu3d_run(cycles);
    }

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
