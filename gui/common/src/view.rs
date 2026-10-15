//! How the two DS screens are arranged in the window (melonDS's View menu):
//! rotation, layout, gap, swap, sizing and integer scaling, and where that
//! puts each screen inside the area the GUI has for them.
//!
//! ```text
//!  View (config.json)          place(view, area)            GUI
//!  rotation / layout / gap --> Placement { screens[top,  --> draws each screen
//!  swap / sizing / integer      bottom]: rect + rotation }   into its rect;
//!                                                            to_ds() maps a
//!                                                            click back to DS
//!                                                            touch coordinates
//! ```
//!
//! Layout is decided after rotation, as in melonDS: each screen is rotated
//! on its own, then the rotated screens are stacked (vertical) or put side
//! by side (horizontal). "Natural" picks the arrangement of a rotated DS:
//! stacked at 0°/180°, side by side at 90°/270°, with the top screen where
//! the console's top screen would end up.
use serde::{Deserialize, Serialize};

/// DS screen size in pixels.
pub const SCREEN_W: f32 = 256.0;
pub const SCREEN_H: f32 = 192.0;

/// Screen rotations offered (degrees clockwise).
pub const ROTATIONS: [u16; 4] = [0, 90, 180, 270];
/// Screen gaps offered (DS pixels), as in melonDS.
pub const SCREEN_GAPS: [u16; 6] = [0, 1, 8, 64, 90, 128];

/// How the screens are arranged.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScreenLayout {
    /// Like a (rotated) DS.
    Natural,
    /// Top screen above the bottom screen.
    #[default]
    #[serde(alias = "Vertical")]
    Vertical,
    /// Top screen left of the bottom screen.
    #[serde(alias = "Horizontal")]
    Horizontal,
}

impl ScreenLayout {
    pub const ALL: [Self; 3] = [Self::Natural, Self::Vertical, Self::Horizontal];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Natural => "Natural",
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
        }
    }
}

/// How big each screen is relative to the other.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ScreenSizing {
    #[default]
    Even,
    /// The top screen twice the size of the bottom one.
    EmphasizeTop,
    /// The bottom screen twice the size of the top one.
    EmphasizeBottom,
    TopOnly,
    BottomOnly,
}

impl ScreenSizing {
    pub const ALL: [Self; 5] = [
        Self::Even,
        Self::EmphasizeTop,
        Self::EmphasizeBottom,
        Self::TopOnly,
        Self::BottomOnly,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Even => "Even",
            Self::EmphasizeTop => "Emphasize top",
            Self::EmphasizeBottom => "Emphasize bottom",
            Self::TopOnly => "Top only",
            Self::BottomOnly => "Bottom only",
        }
    }

    /// Relative size of (top, bottom); 0 hides a screen.
    const fn weights(self) -> (f32, f32) {
        match self {
            Self::Even => (1.0, 1.0),
            Self::EmphasizeTop => (1.0, 0.5),
            Self::EmphasizeBottom => (0.5, 1.0),
            Self::TopOnly => (1.0, 0.0),
            Self::BottomOnly => (0.0, 1.0),
        }
    }
}

/// View settings, saved in each GUI's `config.json` (flattened, so the old
/// `scale` / `layout` / `show_fps` fields keep loading).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct View {
    /// Window size factor chosen with View > Screen size (the screens then
    /// fill whatever room the window has).
    pub scale: f32,
    pub layout: ScreenLayout,
    /// Degrees clockwise, one of [`ROTATIONS`].
    pub rotation: u16,
    /// DS pixels between the screens.
    pub gap: u16,
    /// Swap the top and bottom screen.
    pub swap: bool,
    pub sizing: ScreenSizing,
    /// Only whole-number scale factors (sharp pixels).
    pub integer_scaling: bool,
    /// Smooth (bilinear) instead of nearest-neighbour filtering.
    pub filtering: bool,
    /// Frame rate on screen.
    #[serde(alias = "show_fps")]
    pub show_fps: bool,
    /// Emulation speed in percent, one of [`crate::speed::SPEEDS`]
    /// (Config > Emulation speed, or the left stick button).
    pub speed: u16,
    /// Internal resolution factor (1-4) of the 3D renderer and of the
    /// screen images (Config > Video settings).
    pub render_scale: u8,
    /// xBRZ filter on the 2D graphics (needs `render_scale` >= 2).
    pub xbrz: bool,
    /// Run the enabled Action Replay codes (System > Enable cheats).
    pub cheats_enabled: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            scale: 2.0,
            layout: ScreenLayout::Vertical,
            rotation: 0,
            gap: 8,
            swap: false,
            sizing: ScreenSizing::Even,
            integer_scaling: false,
            filtering: false,
            show_fps: true,
            speed: 100,
            render_scale: 1,
            xbrz: false,
            cheats_enabled: true,
        }
    }
}

impl View {
    /// [`Self::render_scale`] limited to what the renderer supports.
    pub fn render_scale(&self) -> u8 {
        self.render_scale.clamp(1, MAX_RENDER_SCALE)
    }
}

/// Highest internal resolution factor offered.
pub const MAX_RENDER_SCALE: u8 = 4;

/// An axis-aligned rectangle in GUI units.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

/// Where one DS screen is drawn.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    /// The rotated screen's on-screen box.
    pub rect: Rect,
    /// Degrees clockwise the 256x192 image is rotated by inside `rect`.
    pub rotation: u16,
}

/// The result of [`place`]: `[top, bottom]`, `None` for a hidden screen.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub screens: [Option<ScreenRect>; 2],
    /// Size of the whole arrangement at scale 1 (DS pixels), for sizing the
    /// window with View > Screen size.
    pub natural_w: f32,
    pub natural_h: f32,
}

impl View {
    /// Rotation normalised to one of [`ROTATIONS`].
    pub const fn rotation(&self) -> u16 {
        match self.rotation {
            90 | 180 | 270 => self.rotation,
            _ => 0,
        }
    }

    /// Stacked (`true`) or side by side, after rotation.
    const fn stacked(&self) -> bool {
        match self.layout {
            ScreenLayout::Natural => matches!(self.rotation(), 0 | 180),
            ScreenLayout::Vertical => true,
            ScreenLayout::Horizontal => false,
        }
    }

    /// Whether the bottom screen comes first (above / left).
    const fn bottom_first(&self) -> bool {
        // A DS turned 90° clockwise has its top screen on the right; turned
        // 180° the top screen is below.
        let rotated =
            matches!(self.layout, ScreenLayout::Natural) && matches!(self.rotation(), 90 | 180);
        rotated != self.swap
    }
}

/// Places the screens of `view` centred in an `area_w` x `area_h` area.
pub fn place(view: &View, area_w: f32, area_h: f32) -> Placement {
    let rotation = view.rotation();
    let (sw, sh) = if rotation.is_multiple_of(180) {
        (SCREEN_W, SCREEN_H)
    } else {
        (SCREEN_H, SCREEN_W)
    };
    let (wt, wb) = view.sizing.weights();
    let gap = if wt > 0.0 && wb > 0.0 {
        f32::from(view.gap)
    } else {
        0.0
    };
    let stacked = view.stacked();

    // The arrangement at scale 1, in DS pixels.
    let (natural_w, natural_h) = if stacked {
        (sw * wt.max(wb), sh.mul_add(wt + wb, gap))
    } else {
        (sw.mul_add(wt + wb, gap), sh * wt.max(wb))
    };

    let mut factor = (area_w / natural_w).min(area_h / natural_h).max(0.0);
    if view.integer_scaling && factor >= 1.0 {
        factor = factor.floor();
    }
    let (total_w, total_h) = (natural_w * factor, natural_h * factor);
    let (x0, y0) = ((area_w - total_w) / 2.0, (area_h - total_h) / 2.0);

    // First and second screen along the arrangement axis.
    let order: [(usize, f32); 2] = if view.bottom_first() {
        [(1, wb), (0, wt)]
    } else {
        [(0, wt), (1, wb)]
    };
    let mut screens = [None, None];
    let mut along = 0.0;
    for (index, weight) in order {
        if weight <= 0.0 {
            continue;
        }
        let (w, h) = (sw * weight * factor, sh * weight * factor);
        let rect = if stacked {
            Rect {
                x: x0 + (total_w - w) / 2.0,
                y: y0 + along,
                w,
                h,
            }
        } else {
            Rect {
                x: x0 + along,
                y: y0 + (total_h - h) / 2.0,
                w,
                h,
            }
        };
        along += gap.mul_add(factor, if stacked { h } else { w });
        screens[index] = Some(ScreenRect { rect, rotation });
    }
    Placement {
        screens,
        natural_w,
        natural_h,
    }
}

impl ScreenRect {
    /// The DS pixel (0..256, 0..192) under GUI point `(px, py)`, or `None`
    /// outside the screen.
    pub fn to_ds(&self, px: f32, py: f32) -> Option<(i32, i32)> {
        let r = self.rect;
        if !r.contains(px, py) || r.w <= 0.0 || r.h <= 0.0 {
            return None;
        }
        // Normalised position inside the rotated box.
        let (u, v) = ((px - r.x) / r.w, (py - r.y) / r.h);
        // Undo the clockwise rotation.
        let (sx, sy) = match self.rotation {
            90 => (v, 1.0 - u),
            180 => (1.0 - u, 1.0 - v),
            270 => (1.0 - v, u),
            _ => (u, v),
        };
        Some((
            ((sx * SCREEN_W) as i32).clamp(0, 255),
            ((sy * SCREEN_H) as i32).clamp(0, 191),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(p: &Placement, i: usize) -> Rect {
        p.screens[i].map(|s| s.rect).unwrap_or_default()
    }

    #[test]
    fn vertical_fits_and_centres() {
        let view = View {
            gap: 0,
            ..View::default()
        };
        let p = place(&view, 512.0, 384.0_f32.mul_add(2.0, 100.0));
        assert_eq!(
            rect(&p, 0),
            Rect {
                x: 0.0,
                y: 50.0,
                w: 512.0,
                h: 384.0
            }
        );
        assert_eq!(
            rect(&p, 1),
            Rect {
                x: 0.0,
                y: 434.0,
                w: 512.0,
                h: 384.0
            }
        );
        assert_eq!((p.natural_w, p.natural_h), (256.0, 384.0));
    }

    #[test]
    fn swap_gap_sizing_and_integer() {
        let view = View {
            swap: true,
            gap: 8,
            sizing: ScreenSizing::EmphasizeTop,
            integer_scaling: true,
            ..View::default()
        };
        // Natural size 256 x (192 + 96 + 8) = 256 x 296; 2.5x -> 2x.
        let p = place(&view, 640.0, 740.0);
        assert_eq!((p.natural_w, p.natural_h), (256.0, 296.0));
        let (top, bottom) = (rect(&p, 0), rect(&p, 1));
        assert_eq!(
            (top.w, top.h, bottom.w, bottom.h),
            (512.0, 384.0, 256.0, 192.0)
        );
        assert!(bottom.y < top.y, "swapped: bottom screen first");

        let only = View {
            sizing: ScreenSizing::BottomOnly,
            ..View::default()
        };
        let p = place(&only, 256.0, 192.0);
        assert!(p.screens[0].is_none());
        assert_eq!(
            rect(&p, 1),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 256.0,
                h: 192.0
            }
        );
    }

    #[test]
    fn natural_rotation_is_a_turned_ds() {
        let view = View {
            layout: ScreenLayout::Natural,
            rotation: 90,
            gap: 0,
            ..View::default()
        };
        let p = place(&view, 384.0, 256.0);
        let (top, bottom) = (rect(&p, 0), rect(&p, 1));
        assert_eq!((top.w, top.h), (192.0, 256.0));
        assert!(
            top.x > bottom.x,
            "turned clockwise: top screen on the right"
        );
    }

    #[test]
    fn touch_maps_back_through_rotation() {
        let screen = |rotation: u16| ScreenRect {
            rect: Rect {
                x: 10.0,
                y: 20.0,
                w: if rotation.is_multiple_of(180) {
                    256.0
                } else {
                    192.0
                },
                h: if rotation.is_multiple_of(180) {
                    192.0
                } else {
                    256.0
                },
            },
            rotation,
        };
        // DS pixel (0, 0) is the screen's top-left corner before rotation.
        assert_eq!(screen(0).to_ds(10.0, 20.0), Some((0, 0)));
        assert_eq!(screen(90).to_ds(10.0 + 191.5, 20.0), Some((0, 0)));
        assert_eq!(screen(180).to_ds(10.0 + 255.5, 20.0 + 191.5), Some((0, 0)));
        assert_eq!(screen(270).to_ds(10.0, 20.0 + 255.5), Some((0, 0)));
        assert_eq!(screen(0).to_ds(0.0, 0.0), None);
    }

    #[test]
    fn old_configs_still_load() {
        let egui: View =
            serde_json::from_str(r#"{"scale":2.9,"layout":"Horizontal","show_fps":false}"#)
                .unwrap_or_default();
        assert_eq!(
            (egui.scale, egui.layout, egui.show_fps),
            (2.9, ScreenLayout::Horizontal, false)
        );
        let tauri: View =
            serde_json::from_str(r#"{"scale":2.0,"layout":"vertical"}"#).unwrap_or_default();
        assert_eq!(tauri.layout, ScreenLayout::Vertical);
    }
}
