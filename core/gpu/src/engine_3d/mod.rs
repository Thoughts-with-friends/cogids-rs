// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! DS 3D engine: geometry engine ([`geometry`]), registers and buffer swap
//! ([`regs`]) and the software rasterizer ([`raster`]).
pub mod geometry;
pub mod gxfifo;
pub mod raster;
pub mod regs;
pub mod state;

use crate::Gpu;

impl Gpu {
    /// Renders the 3D frame swapped in at the last VBlank. It is shown on
    /// engine A's BG0 during the next frame.
    pub fn render_3d_frame(&mut self) {
        let e = &mut self.engine_3d;
        if !self.engine_upper.dispcnt.bg_3d {
            // Not displayed: skip, and render once 3D is shown again.
            e.render_dirty = true;
            return;
        }
        let bank = e.render_bank;
        e.raster.render(
            &e.render,
            &e.polygon_ram[bank],
            &e.render_polygons,
            &e.vertex_ram[bank],
            &self.vram,
            e.render_dirty,
        );
        e.render_dirty = false;
    }
}
