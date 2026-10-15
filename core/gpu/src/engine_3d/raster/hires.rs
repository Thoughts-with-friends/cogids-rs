// SPDX-License-Identifier: GPL-3.0-or-later
//! High-resolution 3D for display only.
//!
//! The emulated hardware keeps using the native 256x192 raster (BG0,
//! display capture). When an internal resolution above 1x is chosen, the
//! inputs of every natively rendered frame are copied, and the frontend asks
//! for the scaled frame when it draws, so fast-forwarded frames that are
//! never shown cost only that copy.
//!
//! ```text
//!  Gpu::render_3d_frame (line 215)
//!     native Raster::render ---> capture(): render state, polygons, vertices
//!                                          (only when the native frame changed)
//!  frontend draws a frame
//!     Gpu::hires_3d() ---> render(): vertices at hires_position * scale,
//!                          polygon bounds recomputed, same rasterizer at
//!                          scale x 256 x 192, textures from the native raster
//!     display::hires composites it over the upscaled 2D picture
//! ```
use super::Raster;
use crate::engine_3d::state::{Polygon, RenderState, Vertex};

/// Inputs of one native frame.
#[derive(Debug)]
struct Snapshot {
    st: RenderState,
    polys: Vec<Polygon>,
    verts: Vec<Vertex>,
    order: Vec<usize>,
    /// Native raster generation these inputs produced.
    generation: u64,
}

/// High-resolution 3D state (owned by `Gpu3D`).
#[derive(Debug)]
pub struct Hires {
    scale: usize,
    snapshot: Option<Snapshot>,
    raster: Option<Raster>,
    /// Snapshot generation the scaled raster holds.
    rendered: Option<u64>,
}

impl Default for Hires {
    fn default() -> Self {
        Self {
            scale: 1,
            snapshot: None,
            raster: None,
            rendered: None,
        }
    }
}

impl Hires {
    /// The internal resolution factor (1 = off).
    pub const fn scale(&self) -> usize {
        self.scale
    }

    /// Sets the internal resolution factor (1 turns it off).
    pub fn set_scale(&mut self, scale: usize) {
        let scale = scale.clamp(1, 8);
        if scale == self.scale {
            return;
        }
        self.scale = scale;
        self.raster = None;
        self.rendered = None;
        if scale == 1 {
            self.snapshot = None;
        }
    }

    /// Copies the inputs of the frame the native raster holds, if they are
    /// new (or nothing was copied yet).
    pub(crate) fn capture(
        &mut self,
        native: &Raster,
        st: &RenderState,
        polys: &[Polygon],
        order: &[usize],
        verts: &[Vertex],
    ) {
        if self.scale == 1
            || self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.generation == native.generation())
        {
            return;
        }
        let poly_end = order.iter().max().map_or(0, |&i| i + 1).min(polys.len());
        let polys = &polys[..poly_end];
        let vert_end = order
            .iter()
            .filter_map(|&i| polys.get(i))
            .flat_map(|p| p.vertices[..p.num_vertices].iter())
            .max()
            .map_or(0, |&v| v + 1)
            .min(verts.len());
        self.snapshot = Some(Snapshot {
            st: st.clone(),
            polys: polys.to_vec(),
            verts: verts[..vert_end].to_vec(),
            order: order.to_vec(),
            generation: native.generation(),
        });
    }

    /// The scaled 3D frame (rendered now if the native frame changed since).
    /// `None` at 1x or before the first 3D frame.
    pub(crate) fn render(&mut self, native: &Raster) -> Option<&Raster> {
        let snap = self.snapshot.as_ref()?;
        let scale = self.scale;
        if self.rendered != Some(snap.generation) {
            let s = scale as i32;
            let verts: Vec<Vertex> = snap
                .verts
                .iter()
                .map(|v| Vertex {
                    final_position: [
                        (v.hires_position[0] * s) >> 4,
                        (v.hires_position[1] * s) >> 4,
                    ],
                    ..*v
                })
                .collect();
            let polys: Vec<Polygon> = snap
                .polys
                .iter()
                .map(|p| scaled_bounds(p, &verts, s))
                .collect();
            let raster = self.raster.get_or_insert_with(|| Raster::with_scale(scale));
            raster.render_with(
                &snap.st,
                &polys,
                &snap.order,
                &verts,
                native.tex(),
                native.pal(),
            );
            self.rendered = Some(snap.generation);
        }
        self.raster.as_ref()
    }
}

/// `p` with its top / bottom vertex and Y range taken from the scaled
/// vertices (same rules as polygon setup in the geometry engine).
fn scaled_bounds(p: &Polygon, verts: &[Vertex], s: i32) -> Polygon {
    let mut q = *p;
    let (mut ytop, mut ybot, mut xtop, mut xbot) = (192 * s, 0, 256 * s, 0);
    for i in 0..p.num_vertices {
        let Some(v) = verts.get(p.vertices[i]) else {
            q.degenerate = true;
            return q;
        };
        let [fx, fy] = v.final_position;
        if fy < ytop {
            xtop = fx;
            ytop = fy;
            q.v_top = i;
        }
        if fy > ybot || (fy == ybot && fx > xbot) {
            xbot = fx;
            ybot = fy;
            q.v_bottom = i;
        }
    }
    q.y_top = ytop;
    q.y_bottom = ybot;
    q.x_top = xtop;
    q.x_bottom = xbot;
    q
}
