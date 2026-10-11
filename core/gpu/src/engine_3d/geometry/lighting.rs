//! Vertex lighting for the NORMAL command (melonDS `CalculateLighting`).
//! See GBATEK "DS 3D Polygon Light Parameters".
use super::mul;
use crate::engine_3d::state::Gpu3D;

impl Gpu3D {
    /// melonDS `CalculateLighting`.
    pub(super) fn calculate_lighting(&mut self) {
        let n = self.normal.map(i32::from);
        if self.tex_param >> 30 == 2 {
            let m = &self.tex_matrix;
            for i in 0..2 {
                let v = (mul(n[0], m[i]) + mul(n[1], m[4 + i]) + mul(n[2], m[8 + i])) >> 21;
                self.texcoords[i] = self.raw_texcoords[i].wrapping_add(v as i16);
            }
        }

        let m = &self.vec_matrix;
        let nt: [i32; 3] = core::array::from_fn(|c| {
            (n[0]
                .wrapping_mul(m[c])
                .wrapping_add(n[1].wrapping_mul(m[4 + c]))
                .wrapping_add(n[2].wrapping_mul(m[8 + c])))
                << 9
                >> 21
        });

        let mut c = 0_u32;
        let mut buf: [u32; 3] = self.mat_emission.map(|e| u32::from(e) << 14);
        for i in 0..4 {
            if self.cur_polygon_attr & (1 << i) == 0 {
                continue;
            }
            let ld = self.light_direction[i].map(i32::from);
            let lc = self.light_color[i].map(u32::from);
            let mut dot = ((ld[0] * nt[0]) >> 9) + ((ld[1] * nt[1]) >> 9) + ((ld[2] * nt[2]) >> 9);

            let mut shine: i32 = 0;
            if dot > 0 {
                let diff = (dot << 21) >> 21;
                for k in 0..3 {
                    let d = (u32::from(self.mat_diffuse[k]) * lc[k]).wrapping_mul(diff as u32);
                    buf[k] = buf[k].wrapping_add(d & 0xFFFFF);
                }
                dot += nt[2];
                dot = (dot << 21) >> 21;
                dot = ((dot * dot) >> 10) & 0x3FF;
                shine = ((dot * self.spec_recip[i]) >> 8) - (1 << 9);
                if shine < 0 {
                    shine = 0;
                } else {
                    shine = (shine << 18) >> 18;
                    shine = shine.clamp(0, 0x1FF);
                }
            }
            if self.use_shininess_table {
                shine = i32::from(self.shininess_table[(shine >> 2) as usize & 0x7F]) << 1;
            }
            for k in 0..3 {
                let v = (u32::from(self.mat_specular[k]) * shine as u32
                    + (u32::from(self.mat_ambient[k]) << 9))
                    * lc[k];
                buf[k] = buf[k].wrapping_add(v);
            }
            c += 1;
        }
        self.vertex_color = buf.map(|v| (v >> 14).min(31) as u8);
        let _ = c;
    }
}
