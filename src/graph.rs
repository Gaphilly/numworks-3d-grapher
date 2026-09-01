//! Coordinate-system policy: palette, axis visibility, and bounded 1/2/5 ticks.
//!
//! This module generates mathematical choices only. Camera projection and band
//! rasterization remain in their respective layers. All tick production is fixed
//! and capped, preventing very large/fine domains from creating hundreds of lines.

use crate::eadk::Color;
use crate::surface::{Domain, ResolutionPreset};

/// Hard upper bound for tick positions generated on one axis.
pub const MAX_TICKS: usize = 12;

/// Surface rasterization style selected from Settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RenderingMode {
    /// Original low-cost row/column wire mesh.
    Wireframe,
    /// Lit, depth-tested filled triangles.
    Solid,
}

impl RenderingMode {
    /// Cycles forward through the two user-visible modes.
    pub fn next(self) -> RenderingMode {
        match self {
            RenderingMode::Wireframe => RenderingMode::Solid,
            RenderingMode::Solid => RenderingMode::Wireframe,
        }
    }

    /// Cycles backward through the three user-visible modes.
    pub fn previous(self) -> RenderingMode {
        match self {
            RenderingMode::Wireframe => RenderingMode::Solid,
            RenderingMode::Solid => RenderingMode::Wireframe,
        }
    }
}

/// Bounded tone mappings applied to the cached per-triangle diffuse light.
///
/// These presets do not change geometry or relight the surface. They select a
/// flash-resident RGB565 lookup table, so camera redraws retain the same cost.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightingPreset {
    /// Restrained contrast suitable for the default calculator display view.
    Standard,
    /// Higher ambient light for gentler shadows.
    Soft,
    /// Lower ambient light for stronger shape definition.
    Strong,
}

impl LightingPreset {
    pub const COUNT: usize = 3;

    pub fn next(self) -> LightingPreset {
        match self {
            LightingPreset::Standard => LightingPreset::Soft,
            LightingPreset::Soft => LightingPreset::Strong,
            LightingPreset::Strong => LightingPreset::Standard,
        }
    }

    pub fn previous(self) -> LightingPreset {
        match self {
            LightingPreset::Standard => LightingPreset::Strong,
            LightingPreset::Soft => LightingPreset::Standard,
            LightingPreset::Strong => LightingPreset::Soft,
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// An allocation-free RGB888 color used only to configure Solid shading.
///
/// Rendering converts this value into a bounded RGB565 lookup table when the
/// custom color changes. Camera redraws never perform RGB888 conversion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb888 {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb888 {
    /// Initial editable color. It remains independent from the built-in Blue.
    pub const DEFAULT_CUSTOM: Rgb888 = Rgb888 {
        red: 128,
        green: 192,
        blue: 255,
    };

    /// Converts eight-bit channels to RGB565 using deterministic truncation.
    pub const fn to_rgb565(self) -> u16 {
        ((self.red as u16 & 0xf8) << 8)
            | ((self.green as u16 & 0xfc) << 3)
            | ((self.blue as u16) >> 3)
    }

    /// Compact identity used to decide whether the persistent Custom LUT is current.
    pub const fn packed(self) -> u32 {
        ((self.red as u32) << 16) | ((self.green as u32) << 8) | self.blue as u32
    }
}

/// Solid-only base colors. Wireframe deliberately retains its released color.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfacePalette {
    Blue,
    Green,
    Orange,
    Purple,
    Gray,
    Red,
    Cyan,
    Yellow,
    White,
    Custom,
}

impl SurfacePalette {
    pub const BUILTIN_COUNT: usize = 9;

    pub fn next(self) -> SurfacePalette {
        match self {
            SurfacePalette::Blue => SurfacePalette::Green,
            SurfacePalette::Green => SurfacePalette::Orange,
            SurfacePalette::Orange => SurfacePalette::Purple,
            SurfacePalette::Purple => SurfacePalette::Gray,
            SurfacePalette::Gray => SurfacePalette::Red,
            SurfacePalette::Red => SurfacePalette::Cyan,
            SurfacePalette::Cyan => SurfacePalette::Yellow,
            SurfacePalette::Yellow => SurfacePalette::White,
            SurfacePalette::White => SurfacePalette::Custom,
            SurfacePalette::Custom => SurfacePalette::Blue,
        }
    }

    pub fn previous(self) -> SurfacePalette {
        match self {
            SurfacePalette::Blue => SurfacePalette::Custom,
            SurfacePalette::Green => SurfacePalette::Blue,
            SurfacePalette::Orange => SurfacePalette::Green,
            SurfacePalette::Purple => SurfacePalette::Orange,
            SurfacePalette::Gray => SurfacePalette::Purple,
            SurfacePalette::Red => SurfacePalette::Gray,
            SurfacePalette::Cyan => SurfacePalette::Red,
            SurfacePalette::Yellow => SurfacePalette::Cyan,
            SurfacePalette::White => SurfacePalette::Yellow,
            SurfacePalette::Custom => SurfacePalette::White,
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Index into the flash-resident built-in tables, or `None` for Custom.
    pub const fn builtin_index(self) -> Option<usize> {
        if self.index() < Self::BUILTIN_COUNT {
            Some(self.index())
        } else {
            None
        }
    }
}

/// Solid base colors indexed by [`SurfacePalette`]. Tone mapping is performed
/// by compile-time tables in the renderer, not by per-pixel channel arithmetic.
pub const SOLID_SURFACE_COLORS: [u16; SurfacePalette::BUILTIN_COUNT] = [
    0x2d9f, // Blue: released Solid base color.
    0x2da5, // Green.
    0xfd20, // Orange.
    0x881f, // Purple.
    0x9cf3, // Gray.
    0xf800, // Red.
    0x07ff, // Cyan.
    0xffe0, // Yellow.
    0xffff, // White.
];

/// Persistent graph-appearance options. These are tiny value types and changing
/// them never requires expression recompilation or surface resampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphOptions {
    /// Height-field rasterization path.
    pub rendering_mode: RenderingMode,
    /// Solid-only ambient/diffuse response.
    pub lighting: LightingPreset,
    /// Solid-only RGB565 base tint.
    pub surface_palette: SurfacePalette,
    /// User-maintained RGB888 base used when `surface_palette` is Custom.
    pub custom_rgb: Rgb888,
    /// Fixed surface sampling density. Standard is the released 25×19 grid.
    pub resolution: ResolutionPreset,
    /// World-space grid on the XY plane; distinct from the solid surface mesh.
    pub show_grid: bool,
    /// World-space X/Y/Z axes and origin.
    pub show_axes: bool,
    /// Short world-space marks along visible axes.
    pub show_ticks: bool,
    /// Numeric tick values and X/Y/Z bitmap labels.
    pub show_labels: bool,
    /// Shows the latest complete graph redraw duration and derived FPS in Settings.
    pub show_performance: bool,
}

impl GraphOptions {
    /// Released defaults: wireframe with full coordinate context.
    pub const DEFAULT: GraphOptions = GraphOptions {
        rendering_mode: RenderingMode::Wireframe,
        lighting: LightingPreset::Standard,
        surface_palette: SurfacePalette::Blue,
        custom_rgb: Rgb888::DEFAULT_CUSTOM,
        resolution: ResolutionPreset::Standard,
        show_grid: true,
        show_axes: true,
        show_ticks: true,
        show_labels: true,
        show_performance: false,
    };
}

#[derive(Clone, Copy)]
/// Central RGB565 graph palette. Keeping colors here prevents layers from
/// silently diverging as the visual style evolves.
pub struct GraphPalette {
    pub background: Color,
    pub surface: Color,
    pub grid: Color,
    pub x_axis: Color,
    pub y_axis: Color,
    pub z_axis: Color,
    pub origin: Color,
    pub text: Color,
}

/// Restrained default graph colors stored as 16-bit RGB565 values.
pub const PALETTE: GraphPalette = GraphPalette {
    background: Color { rgb565: 0xffff },
    surface: Color { rgb565: 0x001f },
    grid: Color { rgb565: 0xd69a },
    x_axis: Color { rgb565: 0xb800 },
    y_axis: Color { rgb565: 0x05a0 },
    z_axis: Color { rgb565: 0x001f },
    origin: Color { rgb565: 0xfd20 },
    text: Color { rgb565: 0x630c },
};

#[derive(Clone, Copy, Debug, PartialEq)]
/// Axes that mathematically intersect the current XY domain.
pub struct AxisVisibility {
    pub x: bool,
    pub y: bool,
    pub z: bool,
}

/// Determines which world axes can be drawn for a rectangular domain.
pub fn axes_for_domain(domain: Domain) -> AxisVisibility {
    AxisVisibility {
        x: domain.contains_y_zero(),
        y: domain.contains_x_zero(),
        z: domain.contains_x_zero() && domain.contains_y_zero(),
    }
}

#[derive(Clone, Copy)]
/// Allocation-free iterator over a bounded sequence of sensible tick values.
pub struct TickGenerator {
    step: f32,
    next: f32,
    maximum: f32,
    count: usize,
}

impl TickGenerator {
    /// Starts ticks at the first interval multiple inside `[minimum, maximum]`.
    pub fn new(minimum: f32, maximum: f32) -> TickGenerator {
        let step = tick_interval(minimum, maximum);
        let mut first_multiple = (minimum / step) as i32;
        let mut first = first_multiple as f32 * step;
        if first < minimum - step * 0.0001 {
            first_multiple += 1;
            first = first_multiple as f32 * step;
        }
        TickGenerator {
            step,
            next: first,
            maximum,
            count: 0,
        }
    }

    /// Returns the next tick, canonicalizing near-zero values to exactly zero.
    pub fn next(&mut self) -> Option<f32> {
        if self.count >= MAX_TICKS || self.next > self.maximum + self.step * 0.0001 {
            return None;
        }
        let value = self.next;
        self.next += self.step;
        self.count += 1;
        Some(if value.abs() < self.step * 0.0001 {
            0.0
        } else {
            value
        })
    }
}

/// Chooses a `1`, `2`, or `5` times power-of-ten interval targeting ~8 ticks.
pub fn tick_interval(minimum: f32, maximum: f32) -> f32 {
    let span = (maximum - minimum).abs();
    if !span.is_finite() || span <= 0.0 {
        return 1.0;
    }
    let desired = span / 8.0;
    let mut magnitude = 1.0_f32;
    while desired >= magnitude * 10.0 {
        magnitude *= 10.0;
    }
    while desired < magnitude {
        magnitude *= 0.1;
    }
    let normalized = desired / magnitude;
    let factor = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    factor * magnitude
}

#[cfg(test)]
pub fn grid_line_count(domain: Domain) -> usize {
    let mut count = 0;
    let mut x_ticks = TickGenerator::new(domain.x_min, domain.x_max);
    while let Some(value) = x_ticks.next() {
        if value != 0.0 {
            count += 1;
        }
    }
    let mut y_ticks = TickGenerator::new(domain.y_min, domain.y_max);
    while let Some(value) = y_ticks.next() {
        if value != 0.0 {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_only_exist_when_their_zero_coordinate_is_in_domain() {
        let all = axes_for_domain(Domain::new(-1.0, 1.0, -2.0, 2.0));
        assert_eq!(
            all,
            AxisVisibility {
                x: true,
                y: true,
                z: true
            }
        );

        let only_x = axes_for_domain(Domain::new(1.0, 2.0, -2.0, 2.0));
        assert_eq!(
            only_x,
            AxisVisibility {
                x: true,
                y: false,
                z: false
            }
        );
    }

    #[test]
    fn tick_intervals_follow_one_two_five_progression() {
        assert_eq!(tick_interval(-3.1415927, 3.1415927), 1.0);
        assert_eq!(tick_interval(-10.0, 10.0), 5.0);
        assert_eq!(tick_interval(0.0, 1.0), 0.2);
    }

    #[test]
    fn grid_generation_is_bounded_and_skips_axis_duplicates() {
        let domain = Domain::new(-3.1415927, 3.1415927, -3.1415927, 3.1415927);
        assert_eq!(grid_line_count(domain), 12);
        assert!(grid_line_count(Domain::new(-100.0, 100.0, -100.0, 100.0)) <= MAX_TICKS * 2);
    }

    #[test]
    fn rendering_modes_cycle_in_both_directions() {
        assert_eq!(RenderingMode::Wireframe.next(), RenderingMode::Solid);
        assert_eq!(RenderingMode::Solid.next(), RenderingMode::Wireframe);
        assert_eq!(RenderingMode::Wireframe.previous(), RenderingMode::Solid);
    }

    #[test]
    fn rgb888_conversion_uses_bounded_rgb565_channels() {
        assert_eq!(
            Rgb888 {
                red: 0,
                green: 0,
                blue: 0
            }
            .to_rgb565(),
            0x0000
        );
        assert_eq!(
            Rgb888 {
                red: 255,
                green: 255,
                blue: 255
            }
            .to_rgb565(),
            0xffff
        );
        assert_eq!(
            Rgb888 {
                red: 255,
                green: 0,
                blue: 0
            }
            .to_rgb565(),
            0xf800
        );
        assert_eq!(
            Rgb888 {
                red: 0,
                green: 255,
                blue: 0
            }
            .to_rgb565(),
            0x07e0
        );
        assert_eq!(
            Rgb888 {
                red: 0,
                green: 0,
                blue: 255
            }
            .to_rgb565(),
            0x001f
        );
        assert_eq!(
            Rgb888 {
                red: 128,
                green: 192,
                blue: 255
            }
            .to_rgb565(),
            0x861f
        );
    }

    #[test]
    fn graph_options_keep_the_expected_compact_layout() {
        assert_eq!(core::mem::size_of::<Rgb888>(), 3);
        assert_eq!(core::mem::size_of::<GraphOptions>(), 12);
        assert_eq!(GraphOptions::DEFAULT.custom_rgb, Rgb888::DEFAULT_CUSTOM);
    }
}
