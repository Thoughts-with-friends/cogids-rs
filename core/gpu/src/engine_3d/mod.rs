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
    ///
    /// ```text
    /// from: ds_emu gpu_handle_event() at line 215 (after the VBlank
    ///       texture uploads)
    ///   |
    ///   v
    /// render_3d_frame()
    ///   1. 3D not displayed (DISPCNT bit 3)?  mark dirty, skip
    ///   2. raster.render(latched registers, polygon/vertex RAM of the
    ///      render bank, sorted render list, VRAM textures, dirty flag)
    ///   |
    ///   v
    /// to:   compose_scanline() -> render_bg_3d() reads raster.line(y)
    /// ```
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
        e.hires.capture(
            &e.raster,
            &e.render,
            &e.polygon_ram[bank],
            &e.render_polygons,
            &e.vertex_ram[bank],
        );
    }

    /// Sets the internal resolution of the display-only 3D copy (1 = off,
    /// see [`raster::hires`]).
    pub fn set_render_scale(&mut self, scale: usize) {
        self.engine_3d.hires.set_scale(scale);
    }

    /// The 3D frame at the internal resolution, if one is set and 3D was
    /// rendered (renders it now if the native frame changed).
    pub fn hires_3d(&mut self) -> Option<&raster::Raster> {
        let e = &mut self.engine_3d;
        e.hires.render(&e.raster)
    }
}
