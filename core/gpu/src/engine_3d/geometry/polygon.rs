//! Vertex submission and polygon setup: strips, culling, clipping, viewport
//! transform, W normalization and depth (melonDS `SubmitVertex` /
//! `SubmitPolygon`). See GBATEK "DS 3D Polygon Definitions".
use super::clip::clip_polygon;
use super::matrix::transform;
use super::mul;
use crate::engine_3d::state::{Gpu3D, MAX_POLYGONS, MAX_VERTICES, Polygon, Vertex};

impl Gpu3D {
    /// melonDS `SubmitVertex`.
    pub(super) fn submit_vertex(&mut self) {
        self.update_clip_matrix();
        let v = self.cur_vertex.map(i32::from);
        let idx = self.vertex_num_in_poly;
        let mut vt = Vertex {
            position: transform(v, &self.clip_matrix),
            color: self.vertex_color.map(|c| (i32::from(c) << 12) + 0xFFF),
            ..Vertex::default()
        };
        if self.tex_param >> 30 == 3 {
            let m = &self.tex_matrix;
            for i in 0..2 {
                let t = (mul(v[0], m[i]) + mul(v[1], m[4 + i]) + mul(v[2], m[8 + i])) >> 24;
                vt.texcoords[i] = (t as i16).wrapping_add(self.raw_texcoords[i]);
            }
        } else {
            vt.texcoords = self.texcoords;
        }
        self.temp_vertices[idx] = vt;
        self.vertex_num_in_poly += 1;

        match self.polygon_mode {
            0 => {
                if self.vertex_num_in_poly == 3 {
                    self.vertex_num_in_poly = 0;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                }
            }
            1 => {
                if self.vertex_num_in_poly == 4 {
                    self.vertex_num_in_poly = 0;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                }
            }
            2 => {
                if self.num_consecutive_polygons & 1 != 0 {
                    self.temp_vertices.swap(0, 1);
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[1] = self.temp_vertices[2];
                } else if self.vertex_num_in_poly == 3 {
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[0] = self.temp_vertices[1];
                    self.temp_vertices[1] = self.temp_vertices[2];
                }
            }
            _ => {
                if self.vertex_num_in_poly == 4 {
                    self.temp_vertices.swap(2, 3);
                    self.vertex_num_in_poly = 2;
                    self.submit_polygon();
                    self.num_consecutive_polygons += 1;
                    self.temp_vertices[0] = self.temp_vertices[3];
                    self.temp_vertices[1] = self.temp_vertices[2];
                }
            }
        }
    }

    /// melonDS `SubmitPolygon`: culling, clipping, viewport transform and
    /// polygon setup.
    #[expect(clippy::too_many_lines)]
    pub(super) fn submit_polygon(&mut self) {
        let mut nverts = if self.polygon_mode & 1 != 0 { 4 } else { 3 };
        let attr = self.cur_polygon_attr;

        // Culling (facing test in homogeneous coordinates).
        let [v0, v1, v2, _] = self.temp_vertices;
        // Differences wrap in 32 bits, products are 64-bit (as in melonDS).
        let d =
            |a: &Vertex, b: &Vertex, i: usize| i64::from(a.position[i].wrapping_sub(b.position[i]));
        let cross = |a0: i64, b0: i64, a1: i64, b1: i64| (a0 * b0).wrapping_sub(a1 * b1);
        let mut nx = cross(
            d(&v0, &v1, 1),
            d(&v2, &v1, 3),
            d(&v0, &v1, 3),
            d(&v2, &v1, 1),
        );
        let mut ny = cross(
            d(&v0, &v1, 3),
            d(&v2, &v1, 0),
            d(&v0, &v1, 0),
            d(&v2, &v1, 3),
        );
        let mut nz = cross(
            d(&v0, &v1, 0),
            d(&v2, &v1, 1),
            d(&v0, &v1, 1),
            d(&v2, &v1, 0),
        );
        let big = |n: i64| (n >> 31) ^ (n >> 63) != 0;
        while big(nx) || big(ny) || big(nz) {
            nx >>= 4;
            ny >>= 4;
            nz >>= 4;
        }
        let dot = i64::from(v1.position[0])
            .wrapping_mul(nx)
            .wrapping_add(i64::from(v1.position[1]).wrapping_mul(ny))
            .wrapping_add(i64::from(v1.position[3]).wrapping_mul(nz));
        let facing_view = dot <= 0;
        if (dot < 0 && attr & (1 << 7) == 0) || (dot > 0 && attr & (1 << 6) == 0) {
            self.last_strip_polygon = None;
            return;
        }

        // Strips: reuse the two shared vertices of the previous polygon.
        let bank = self.cur_bank;
        let mut clipped = [Vertex::default(); 10];
        let mut clip_start = 0;
        let mut reused = [0usize; 2];
        let mut last_poly_verts = 0;
        if self.polygon_mode >= 2
            && let Some(last) = self.last_strip_polygon
        {
            let (id0, id1) = if self.polygon_mode == 2 {
                last_poly_verts = 3;
                if self.num_consecutive_polygons & 1 != 0 {
                    (2, 1)
                } else {
                    (0, 2)
                }
            } else {
                last_poly_verts = 4;
                (3, 2)
            };
            let lp = &self.polygon_ram[bank][last];
            let vr = &self.vertex_ram[bank];
            if lp.num_vertices == last_poly_verts
                && !vr[lp.vertices[id0]].clipped
                && !vr[lp.vertices[id1]].clipped
            {
                reused = [lp.vertices[id0], lp.vertices[id1]];
                clipped[0] = vr[reused[0]];
                clipped[1] = vr[reused[1]];
                clip_start = 2;
            }
        }
        clipped[clip_start..nverts].copy_from_slice(&self.temp_vertices[clip_start..nverts]);

        nverts = clip_polygon(
            true,
            attr & (1 << 12) != 0,
            &mut clipped,
            nverts,
            clip_start,
        );
        if nverts == 0 {
            self.last_strip_polygon = None;
            return;
        }
        if self.num_polygons >= MAX_POLYGONS || self.num_vertices + nverts > MAX_VERTICES {
            self.last_strip_polygon = None;
            self.disp_cnt |= 1 << 13;
            return;
        }

        // Viewport transform (32-bit divider, as on hardware).
        let vp = self.viewport;
        for vtx in &mut clipped[clip_start..nverts] {
            vtx.position[3] &= 0x00FF_FFFF;
            let w = vtx.position[3] as u32;
            let (pos_x, pos_y) = if w == 0 {
                (0, 0)
            } else {
                let mut px = (vtx.position[0] as u32).wrapping_add(w);
                let mut py = (vtx.position[1] as u32).wrapping_neg().wrapping_add(w);
                let mut den = w;
                if w > 0xFFFF {
                    px >>= 1;
                    py >>= 1;
                    den >>= 1;
                }
                den <<= 1;
                (
                    (px.wrapping_mul(vp.width as u32) / den).wrapping_add(vp.x0 as u32),
                    (py.wrapping_mul(vp.height as u32) / den).wrapping_add(vp.y1 as u32),
                )
            };
            vtx.final_position = [(pos_x & 0x1FF) as i32, (pos_y & 0xFF) as i32];
        }

        // Zero-dot polygons far behind are rejected unless bit 13 is set.
        if attr & (1 << 13) == 0 {
            let first = clipped[0].final_position;
            let mut zero_dot = true;
            let mut all_behind = true;
            for v in &clipped[..nverts] {
                if v.final_position != first {
                    zero_dot = false;
                    break;
                }
                if v.position[3] <= self.zero_dot_w_limit {
                    all_behind = false;
                    break;
                }
            }
            if zero_dot && all_behind {
                self.last_strip_polygon = None;
                return;
            }
        }

        let tex_fmt = (self.tex_param >> 26) & 7;
        let alpha = (attr >> 16) & 0x1F;
        let mut poly = Polygon {
            attr,
            tex_param: self.tex_param,
            tex_palette: self.tex_palette,
            facing_view,
            translucent: tex_fmt == 1 || tex_fmt == 6 || (alpha > 0 && alpha < 31),
            is_shadow_mask: attr & 0x3F00_0030 == 0x30,
            ..Polygon::default()
        };
        poly.is_shadow = attr & 0x30 == 0x30 && !poly.is_shadow_mask;

        let vram = &mut self.vertex_ram[bank];
        if self.last_strip_polygon.is_some() && clip_start > 0 {
            if nverts == last_poly_verts {
                poly.vertices[0] = reused[0];
                poly.vertices[1] = reused[1];
            } else {
                for k in 0..2 {
                    vram[self.num_vertices] = vram[reused[k]];
                    poly.vertices[k] = self.num_vertices;
                    self.num_vertices += 1;
                }
            }
            poly.num_vertices = 2;
        }
        for (i, src) in clipped.iter().enumerate().take(nverts).skip(clip_start) {
            let mut vtx = *src;
            for k in 0..3 {
                let c = vtx.color[k] >> 12;
                vtx.final_color[k] = if c != 0 { (c << 4) + 0xF } else { 0 };
            }
            vram[self.num_vertices] = vtx;
            poly.vertices[i] = self.num_vertices;
            self.num_vertices += 1;
            poly.num_vertices += 1;
        }

        // Bounds, W normalization and depth values.
        let (mut ytop, mut ybot, mut xtop, mut xbot) = (192, 0, 256, 0);
        let mut wsize = 0u32;
        for i in 0..nverts {
            let v = &vram[poly.vertices[i]];
            let [fx, fy] = v.final_position;
            if fy < ytop {
                xtop = fx;
                ytop = fy;
                poly.v_top = i;
            }
            if fy > ybot || (fy == ybot && fx > xbot) {
                xbot = fx;
                ybot = fy;
                poly.v_bottom = i;
            }
            let w = v.position[3] as u32;
            if w == 0 {
                poly.degenerate = true;
            }
            while wsize < 32 && (w >> wsize) != 0 {
                wsize += 4;
            }
        }
        poly.y_top = ytop;
        poly.y_bottom = ybot;
        poly.x_top = xtop;
        poly.x_bottom = xbot;
        if ybot > 192 {
            poly.degenerate = true;
        }
        poly.sort_key = ((ybot as u32) << 8) | ytop as u32;
        if poly.translucent {
            poly.sort_key |= 0x10000;
        }
        poly.w_buffer = self.flush_attributes & 2 != 0;

        for i in 0..nverts {
            let v = &vram[poly.vertices[i]];
            let (w, wshifted) = if wsize < 16 {
                let w = v.position[3] << (16 - wsize);
                (w, w >> (16 - wsize))
            } else {
                let w = v.position[3] >> (wsize - 16);
                (w, w << (wsize - 16))
            };
            let z = if poly.w_buffer {
                i64::from(wshifted)
            } else if v.position[3] != 0 {
                (i64::from(v.position[2]) * 0x4000 / i64::from(v.position[3]) + 0x3FFF) * 0x200
            } else {
                0x7F_FE00
            };
            poly.final_z[i] = z.clamp(0, 0xFF_FFFF) as i32;
            poly.final_w[i] = w;
        }

        if !poly.translucent {
            self.num_opaque_polygons += 1;
        }
        let index = self.num_polygons;
        self.polygon_ram[bank][index] = poly;
        self.num_polygons += 1;
        self.last_strip_polygon = (self.polygon_mode >= 2).then_some(index);
    }
}
