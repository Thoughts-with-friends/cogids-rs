// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp
//!

use crate::gpu_3d::structs::{Gpu3D, Vertex};

impl Gpu3D {
    /// Perspective-Correct Linear Interpolation
    pub(crate) fn interpolate(
        &self,
        step: u64,
        step_count: u64,
        value_start: i64,
        value_end: i64,
        depth_scale_start: i32,
        depth_scale_end: i32,
    ) -> i64 {
        // 128-bit: value * depth can exceed 64 bits for extreme geometry.
        let pixel = i128::from(step);
        let pixel_range = i128::from(step_count);
        let w1 = i128::from(depth_scale_start);
        let w2 = i128::from(depth_scale_end);

        let bark = (pixel_range - pixel) * (i128::from(value_start) * w2)
            + pixel * (i128::from(value_end) * w1);
        let denom = (pixel_range - pixel) * w2 + pixel * w1;

        if denom == 0 {
            return value_start;
        }
        (bark / denom) as i64
    }

    // Moved to impl Matrix
    // fn get_identity_matrix(&mut self, mtx: &mut Matrix);
    // pub fn read_command(&mut self) -> Option<GxCommand>;
    // fn write_command(&mut self, cmd: GxCommand);
    // fn exec_command(&mut self)

    pub fn add_mult_param(&mut self, word: u32) {
        let div = self.mult_params_index / 4;
        let modulo = self.mult_params_index % 4;
        self.mult_params.m[div][modulo] = word as i32;
        self.mult_params_index += 1;
    }

    // Moved to execute.rs
    // pub fn mtx_mult(&mut self)

    pub fn update_clip_matrix(&mut self) {
        // if self.clip_dirty || true {
        if self.clip_dirty {
            for i in 0..4 {
                for j in 0..4 {
                    let mut temp_calc = 0_i64;
                    for k in 0..4 {
                        temp_calc += i64::from(self.modelview_mtx.m[i][k])
                            * i64::from(self.projection_mtx.m[k][j]);
                    }
                    self.clip_mtx.m[i][j] = (temp_calc >> 12) as i32;
                }
            }
            self.clip_dirty = false;
        }
    }

    pub fn clip(
        &mut self,
        v_list: &mut [Vertex],
        mut v_len: usize,
        clip_start: usize,
        add_attributes: bool,
    ) -> usize {
        v_len = self.clip_plane(0, v_list, v_len, clip_start, add_attributes);
        v_len = self.clip_plane(1, v_list, v_len, clip_start, add_attributes);
        v_len = self.clip_plane(2, v_list, v_len, clip_start, add_attributes);
        v_len
    }

    pub fn clip_plane(
        &mut self,
        plane: usize,
        v_list: &mut [Vertex],
        mut v_len: usize,
        clip_start: usize,
        add_attributes: bool,
    ) -> usize {
        let mut temp_v_list: Vec<Vertex> = vec![Vertex::default(); 10];
        let mut clip_index = clip_start;

        if clip_start == 2 {
            temp_v_list[0] = v_list[0].clone();
            temp_v_list[1] = v_list[1].clone();
        }

        // Clip everything higher than w
        for i in clip_start..v_len {
            let prev_v = if i == 0 { v_len - 1 } else { i - 1 };
            let next_v = if i + 1 >= v_len { 0 } else { i + 1 };

            let v = v_list[i].clone();

            if v.coords[plane] > v.coords[3] {
                if plane == 2 && !self.current_poly_attr.render_far_intersect {
                    return 0;
                }

                let vp = v_list[prev_v].clone();
                if vp.coords[plane] <= vp.coords[3] {
                    self.clip_vertex(
                        plane,
                        &mut temp_v_list[clip_index],
                        &v,
                        &vp,
                        1,
                        add_attributes,
                    );
                    clip_index += 1;
                }

                let vn = v_list[next_v].clone();
                if vn.coords[plane] <= vn.coords[3] {
                    self.clip_vertex(
                        plane,
                        &mut temp_v_list[clip_index],
                        &v,
                        &vn,
                        1,
                        add_attributes,
                    );
                    clip_index += 1;
                }
            } else {
                temp_v_list[clip_index] = v;
                clip_index += 1;
            }
        }

        v_len = clip_index;
        clip_index = clip_start;

        // Clip everything lower than -w
        for i in clip_start..v_len {
            let prev_v = if i == 0 { v_len - 1 } else { i - 1 };
            let next_v = if i + 1 >= v_len { 0 } else { i + 1 };

            let v = temp_v_list[i].clone();

            if i64::from(v.coords[plane]) < -i64::from(v.coords[3]) {
                let vp = temp_v_list[prev_v].clone();
                if i64::from(vp.coords[plane]) >= -i64::from(vp.coords[3]) {
                    self.clip_vertex(plane, &mut v_list[clip_index], &v, &vp, -1, add_attributes);
                    clip_index += 1;
                }

                let vn = temp_v_list[next_v].clone();
                if i64::from(vn.coords[plane]) >= -i64::from(vn.coords[3]) {
                    self.clip_vertex(plane, &mut v_list[clip_index], &v, &vn, -1, add_attributes);
                    clip_index += 1;
                }
            } else {
                v_list[clip_index] = v;
                clip_index += 1;
            }
        }

        clip_index
    }

    pub fn clip_vertex(
        &mut self,
        plane: usize,
        v_list: &mut Vertex, // output
        v_out: &Vertex,      // outside vertex
        v_in: &Vertex,       // inside vertex
        side: i32,
        add_attributes: bool,
    ) {
        // Copied logic from melonDS version
        let factor_num: i64 = v_in.coords[3] as i64 - (side as i64 * v_in.coords[plane] as i64);

        let factor_den: i64 =
            factor_num - (v_out.coords[3] as i64 - (side as i64 * v_out.coords[plane] as i64));

        // Degenerate edge (both ends on the plane): keep the inside vertex.
        // This used to `panic!`, which stopped emulation on such geometry.
        let interp = |vin: i32, vout: i32| -> i32 {
            if factor_den == 0 {
                return vin;
            }
            (i128::from(vin)
                + (i128::from(vout) - i128::from(vin)) * i128::from(factor_num)
                    / i128::from(factor_den)) as i32
        };

        if plane != 0 {
            v_list.coords[0] = interp(v_in.coords[0], v_out.coords[0]);
        }
        if plane != 1 {
            v_list.coords[1] = interp(v_in.coords[1], v_out.coords[1]);
        }
        if plane != 2 {
            v_list.coords[2] = interp(v_in.coords[2], v_out.coords[2]);
        }

        v_list.coords[3] = interp(v_in.coords[3], v_out.coords[3]);
        v_list.coords[plane] = side.wrapping_mul(v_list.coords[3]);

        if add_attributes {
            v_list.colors[0] = interp(v_in.colors[0], v_out.colors[0]);
            v_list.colors[1] = interp(v_in.colors[1], v_out.colors[1]);
            v_list.colors[2] = interp(v_in.colors[2], v_out.colors[2]);

            v_list.texcoords[0] = interp(v_in.texcoords[0], v_out.texcoords[0]);
            v_list.texcoords[1] = interp(v_in.texcoords[1], v_out.texcoords[1]);
        }

        v_list.clipped = true;
    }

    pub fn add_vertex(&mut self) {
        if self.geo_vert_count >= 6188 {
            return;
        }

        let coords = [
            self.current_vertex[0] as i32,
            self.current_vertex[1] as i32,
            self.current_vertex[2] as i32,
            0x1000,
        ];

        self.update_clip_matrix();

        let index = self.vertex_list_count as usize;
        let mut vtx = self.vertex_list[index].clone();

        for i in 0..4 {
            let m = &self.clip_mtx.m;
            vtx.coords[i] = ((0..4)
                .map(|k| i64::from(coords[k]) * i64::from(m[k][i]))
                .sum::<i64>()
                >> 12) as i32;
        }

        if self.teximage_param.transformation_mode == 3 {
            let texcoords = self.current_texcoords;
            let texture_mtx_i32 = self.texture_mtx.m;

            self.current_texcoords[0] = (((0..3)
                .map(|k| i64::from(coords[k]) * i64::from(texture_mtx_i32[k][0]))
                .sum::<i64>()
                >> 24) as i16)
                .wrapping_add(texcoords[0]);

            self.current_texcoords[1] = (((0..3)
                .map(|k| i64::from(coords[k]) * i64::from(texture_mtx_i32[k][1]))
                .sum::<i64>()
                >> 24) as i16)
                .wrapping_add(texcoords[1]);
        }

        vtx.colors[0] = (((self.current_color & 0x1F) << 12) + 0xFFF) as i32;
        vtx.colors[1] = ((((self.current_color >> 5) & 0x1F) << 12) + 0xFFF) as i32;
        vtx.colors[2] = ((((self.current_color >> 10) & 0x1F) << 12) + 0xFFF) as i32;
        vtx.texcoords[0] = i32::from(self.current_texcoords[0]);
        vtx.texcoords[1] = i32::from(self.current_texcoords[1]);
        vtx.clipped = false;
        // The transformed vertex was computed but never stored, so no
        // polygon ever got real coordinates.
        self.vertex_list[index] = vtx;

        self.vertex_list_count += 1;

        match self.polygon_type {
            0 => {
                let is_3 = self.vertex_list_count == 3;
                if is_3 {
                    self.add_polygon();
                    self.consecutive_polygons += 1;
                    self.vertex_list_count = 0;
                }
            }
            1 => {
                let is_4 = self.vertex_list_count == 4;
                if is_4 {
                    self.add_polygon();
                    self.consecutive_polygons += 1;
                    self.vertex_list_count = 0;
                }
            }
            // Triangle strip: every new vertex forms a triangle with the
            // previous two (the winding of odd triangles is flipped in
            // `add_polygon`).
            2 => {
                if self.vertex_list_count == 3 {
                    self.add_polygon();
                    self.consecutive_polygons += 1;
                    self.vertex_list_count = 2;
                    self.vertex_list[0] = self.vertex_list[1].clone();
                    self.vertex_list[1] = self.vertex_list[2].clone();
                }
            }
            // Quad strip: vertices 0,1,2,3 form the quad 0-1-3-2; the next
            // quad starts from 2,3.
            3 => {
                if self.vertex_list_count == 4 {
                    self.vertex_list.swap(2, 3);
                    self.add_polygon();
                    self.consecutive_polygons += 1;
                    self.vertex_list_count = 2;
                    self.vertex_list[0] = self.vertex_list[3].clone();
                    self.vertex_list[1] = self.vertex_list[2].clone();
                }
            }
            _ => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized polygon_type: {}", self.polygon_type);
            }
        }
    }

    pub fn add_polygon(&mut self) {
        if self.vertex_list_count < 3 || self.vertex_list_count > 4 {
            #[cfg(feature = "tracing")]
            tracing::error!("Error: add_polygon called with invalid vertex_list_count");
        }

        // Cull front/back face polygons. Odd triangles of a strip have
        // their winding reversed.
        let flip = self.polygon_type == 2 && self.consecutive_polygons & 1 != 0;
        let (v0, v1, v2) = if flip {
            (
                &self.vertex_list[1],
                &self.vertex_list[0],
                &self.vertex_list[2],
            )
        } else {
            (
                &self.vertex_list[0],
                &self.vertex_list[1],
                &self.vertex_list[2],
            )
        };

        // Facing test (melonDS `GPU3D::SubmitPolygon`), in 128 bits: vertex
        // differences reach 2^33, so 64-bit products can overflow.
        let c = |v: &Vertex, i: usize| i128::from(v.coords[i]);
        let (ax, ay, az) = (
            c(v0, 0) - c(v1, 0),
            c(v0, 1) - c(v1, 1),
            c(v0, 3) - c(v1, 3),
        );
        let (bx, by, bz) = (
            c(v2, 0) - c(v1, 0),
            c(v2, 1) - c(v1, 1),
            c(v2, 3) - c(v1, 3),
        );
        let mut normal = [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx];
        while normal.iter().any(|&n| (n >> 31) != (n >> 127)) {
            for n in &mut normal {
                *n >>= 4;
            }
        }
        let dot = c(v1, 0) * normal[0] + c(v1, 1) * normal[1] + c(v1, 3) * normal[2];
        let front_view = dot < 0;

        let render = match front_view {
            true => self.current_poly_attr.render_front,
            false => self.current_poly_attr.render_back,
        };

        if !render {
            return;
        }

        if self.geo_poly_count >= 2048 {
            // geo_poly_count++;
            self.disp3dcnt.ram_overflow = true;
            return;
        }

        // Clip the fuck out of that polygon
        let clip_start = 0;
        let mut clipped_count = self.vertex_list_count as usize;
        // let reused_list: Vec<Vertex>; // [2]

        // Attempt to attach vertices from last strip polygon to new one, if possible
        // if self.polygon_type >= 2 {
        //     if let Some(index) = self.last_poly_strip {
        //         let (mut v0, mut v1) = (0, 0);
        //         let mut vertices = 0;

        //         if self.polygon_type == 2 {
        //             if (self.consecutive_polygons & 0x1) != 0 {
        //                 (v0, v1) = (2, 1);
        //             } else {
        //                 (v0, v1) = (0, 2);
        //             }
        //             vertices = 3;
        //         } else {
        //             (v0, v1) = (3, 2);
        //             vertices = 4;
        //         }

        //         let last_poly_strip = &self.geo_poly[index];

        //         // NOTE: Unused in C++.
        //         // if last_poly_strip.vertices == vertices
        //         //     && !self.geo_vert[v + v0].clipped
        //         //     && !self.geo_vert[v + v1].clipped
        //         // {
        //         //     reused_list[0] = self.geo_vert[v + v0];
        //         //     reused_list[1] = self.geo_vert[v + v1];
        //         //     clipped_list[0] = self.geo_vert[v + v0];
        //         //     clipped_list[1] = self.geo_vert[v + v1];
        //         //     clip_start = 2;
        //         // }
        //     }
        // }

        // Clipping against the 6 planes can add vertices: up to 10.
        let mut clipped_list = vec![Vertex::default(); 10];
        clipped_list[..clipped_count].clone_from_slice(&self.vertex_list[..clipped_count]);
        if flip {
            clipped_list.swap(0, 1);
        }
        clipped_count = self.clip(&mut clipped_list, clipped_count, clip_start, true);
        // Fully clipped away (the check was inverted, dropping every visible
        // polygon instead).
        if clipped_count == 0 {
            return;
        }

        // Time to make a polygon!
        // Also normalize w

        for (i, clipped) in clipped_list[0..clipped_count].iter().enumerate() {
            let v = i + self.geo_vert_count as usize;
            if v >= 6188 {
                // #[cfg(feature = "tracing")]
                // tracing::debug!("Vertex count exceeded!");
                self.disp3dcnt.ram_overflow = true;
                return;
            }
            self.geo_vert[v] = clipped.clone();

            // Convert z values
            // final_Z = (((vertexZ * 0x4000) / vertexW) + 0x3FFF) * 0x200
            let z = self.geo_vert[v].coords[2];
            let w = self.geo_vert[v].coords[3];
            if w > 0 {
                let z = (z as i64) * 0x4000;
                let w = w as i64;
                self.geo_vert[v].coords[2] = (((z / w) + 0x3FFF) * 0x200) as i32;
            } else {
                self.geo_vert[v].coords[2] = 0x7FFE00;
            }

            self.geo_vert[v].coords[2] = self.geo_vert[v].coords[2].clamp(0, 0xFFFFFF);

            if self.geo_vert[v].coords[2] == 0xFFFFFF {
                // #[cfg(feature = "tracing")]
                // tracing::debug!("Poly{} max z!", self.geo_poly_count);
            }

            // NOTE: Unused in C++.
            // if (w_len < 16) {
            //     self.geo_vert[v].coords[3] >>= (16 - w_len);
            //     self.geo_vert[v].coords[3] <<= (16 - w_len);
            // } else {
            //     self.geo_vert[v].coords[3] >>= (w_len - 16);
            //     self.geo_vert[v].coords[3] <<= (w_len - 16);
            // }

            let vert_colors = self.geo_vert[v].colors;
            for (color, vert_color) in self.geo_vert[v].final_colors.iter_mut().zip(vert_colors) {
                *color = vert_color >> 12;
                if vert_color > 0 {
                    *color <<= 4;
                    *color += 0xF;
                }
            }
        }

        let index = self.geo_poly_count as usize;

        // Update Polygon data
        self.geo_poly[index].vertices = clipped_count as u8;
        self.geo_poly[index].vert_index = self.geo_vert_count as u16;
        self.geo_poly[index].attributes = self.current_poly_attr.clone();
        self.geo_poly[index].texparams = self.teximage_param.clone();
        self.geo_poly[index].palette_base = self.pltt_base;

        if (self.current_poly_attr.alpha > 0 && self.current_poly_attr.alpha < 0x1F)
            || self.teximage_param.format == 1
            || self.teximage_param.format == 6
        {
            let index = self.geo_poly_count as usize;
            self.geo_poly[index].translucent = true;
        } else {
            let index = self.geo_poly_count as usize;
            self.geo_poly[index].translucent = false;
        }
        self.geo_poly_count += 1;
        self.geo_vert_count += clipped_count as i32;

        if self.polygon_type >= 2 {
            self.vertex_list_count = 2;
            let index = (self.geo_poly_count - 1) as usize;
            self.last_poly_strip = Some(index);
        } else {
            self.last_poly_strip = None;
        }
    }

    // moved gpu_3d.rs in lunaris_emu/gpu
    // pub fn request_fifo_dma(&mut self)
}
