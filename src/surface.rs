//! Mathematical domain and fixed-capacity surface sampling.
//!
//! The renderer supports a bounded set of regular grids. Heights plus the exact sampled X/Y
//! coordinates are cached so Solid camera redraws never repeat domain divisions.
//! Solid-only triangle lighting is rebuilt only when sampling changes, avoiding
//! normal/square-root work while orbiting. Wireframe deliberately retains its
//! established point-reconstruction path and does not consume either cache.
//! Equation bytecode is evaluated only after an expression/domain change, never
//! for a camera-only redraw.

use crate::function::SurfaceFunction;
use crate::math;

/// Fixed graph sampling choices. The largest capacity is allocated once; only
/// the active rectangle is sampled and rendered.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResolutionPreset {
    Low,
    Standard,
    High,
}

impl ResolutionPreset {
    pub const fn columns(self) -> usize {
        match self {
            Self::Low => 17,
            Self::Standard => 25,
            Self::High => 33,
        }
    }

    pub const fn rows(self) -> usize {
        match self {
            Self::Low => 13,
            Self::Standard => 19,
            Self::High => 25,
        }
    }

    #[cfg(test)]
    pub const fn triangle_count(self) -> usize {
        (self.columns() - 1) * (self.rows() - 1) * TRIANGLES_PER_CELL
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Low => Self::Standard,
            Self::Standard => Self::High,
            Self::High => Self::Low,
        }
    }

    pub const fn previous(self) -> Self {
        match self {
            Self::Low => Self::High,
            Self::Standard => Self::Low,
            Self::High => Self::Standard,
        }
    }
}

/// Released Standard dimensions, retained for v2.5 reference tests.
#[cfg(test)]
pub const COLUMNS: usize = ResolutionPreset::Standard.columns();
/// Released Standard dimensions, retained for v2.5 reference tests.
#[cfg(test)]
pub const ROWS: usize = ResolutionPreset::Standard.rows();
/// Maximum fixed storage dimensions; no user input can change them.
pub const MAX_COLUMNS: usize = ResolutionPreset::High.columns();
/// Maximum fixed storage dimensions; no user input can change them.
pub const MAX_ROWS: usize = ResolutionPreset::High.rows();
/// Two consistently wound triangles cover every regular-grid cell.
pub const TRIANGLES_PER_CELL: usize = 2;
/// Transient one-byte light/validity values for every regular-grid triangle.
/// Zero means invalid; values 1..=255 are quantized diffuse illumination.
pub type TriangleShades = [[[u8; TRIANGLES_PER_CELL]; MAX_COLUMNS - 1]; MAX_ROWS - 1];
const MIN_DOMAIN_SPAN: f32 = 0.01;
const MAX_DOMAIN_ABSOLUTE_BOUND: f32 = 1_000.0;
const MAX_DOMAIN_SPAN: f32 = 1_000.0;

/// Why a candidate domain cannot safely replace the active graph bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DomainError {
    /// At least one bound is NaN or infinite.
    NonFinite,
    /// A minimum is greater than or equal to its maximum.
    Inverted,
    /// At least one axis span is below the useful sampling threshold.
    TooNarrow,
    /// A bound or span exceeds the calculator-oriented safety limit.
    TooLarge,
}

/// Inclusive rectangular mathematical domain sampled by the surface and used by
/// axes, grid lines, and ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    /// Inclusive low X bound.
    pub x_min: f32,
    /// Inclusive high X bound.
    pub x_max: f32,
    /// Inclusive low Y bound.
    pub y_min: f32,
    /// Inclusive high Y bound.
    pub y_max: f32,
}

impl Domain {
    /// Initial domain, matching the original hard-coded `[-pi, pi]` range.
    pub const DEFAULT: Domain = Domain::new(-3.1415927, 3.1415927, -3.1415927, 3.1415927);

    /// Creates a domain. Callers that later expose editable bounds must validate
    /// finite, increasing limits before replacing the active domain.
    pub const fn new(x_min: f32, x_max: f32, y_min: f32, y_max: f32) -> Domain {
        Domain {
            x_min,
            x_max,
            y_min,
            y_max,
        }
    }

    /// Validates user-edited bounds without mutating the active domain.
    ///
    /// The limits prevent near-zero sampling steps and ranges so extreme that
    /// tick generation/projection cease to be useful on a 320-pixel display.
    pub fn validate(self) -> Result<(), DomainError> {
        if !self.x_min.is_finite()
            || !self.x_max.is_finite()
            || !self.y_min.is_finite()
            || !self.y_max.is_finite()
        {
            return Err(DomainError::NonFinite);
        }
        if self.x_min >= self.x_max || self.y_min >= self.y_max {
            return Err(DomainError::Inverted);
        }
        let x_span = self.x_max - self.x_min;
        let y_span = self.y_max - self.y_min;
        if x_span < MIN_DOMAIN_SPAN || y_span < MIN_DOMAIN_SPAN {
            return Err(DomainError::TooNarrow);
        }
        if self.x_min.abs() > MAX_DOMAIN_ABSOLUTE_BOUND
            || self.x_max.abs() > MAX_DOMAIN_ABSOLUTE_BOUND
            || self.y_min.abs() > MAX_DOMAIN_ABSOLUTE_BOUND
            || self.y_max.abs() > MAX_DOMAIN_ABSOLUTE_BOUND
            || x_span > MAX_DOMAIN_SPAN
            || y_span > MAX_DOMAIN_SPAN
        {
            return Err(DomainError::TooLarge);
        }
        Ok(())
    }

    /// Whether the world X axis (`y = 0`) crosses this domain.
    pub fn contains_x_zero(self) -> bool {
        self.x_min <= 0.0 && self.x_max >= 0.0
    }

    /// Whether the world Y axis (`x = 0`) crosses this domain.
    pub fn contains_y_zero(self) -> bool {
        self.y_min <= 0.0 && self.y_max >= 0.0
    }

    /// Converts an inclusive column index to world `x`.
    pub fn sample_x(self, column: usize, columns: usize) -> f32 {
        sample(self.x_min, self.x_max, column, columns)
    }

    /// Converts an inclusive row index to world `y`.
    pub fn sample_y(self, row: usize, rows: usize) -> f32 {
        sample(self.y_min, self.y_max, row, rows)
    }
}

/// World-space point. The graph convention is `z = f(x, y)`.
#[derive(Clone, Copy)]
pub struct Point3 {
    /// Mathematical X coordinate.
    pub x: f32,
    /// Mathematical Y coordinate.
    pub y: f32,
    /// Surface height `f(x, y)`.
    pub z: f32,
}

/// Fixed-capacity height cache for the current compiled expression and domain.
///
/// Capacity is sized once for the High 33×25 preset: 3,300 bytes of heights,
/// 232 bytes of cached X/Y coordinates, and 1,536 bytes of triangle light
/// levels. Only the active preset rectangle is sampled or traversed. NaN and
/// infinite evaluations remain in the active height cache and later invalidate
/// every touching Solid triangle, so ordinary invalid mathematical input cannot
/// reach rasterization or bridge a discontinuity.
pub struct SurfaceGrid {
    heights: [[f32; MAX_COLUMNS]; MAX_ROWS],
    sample_x: [f32; MAX_COLUMNS],
    sample_y: [f32; MAX_ROWS],
    triangle_shades: TriangleShades,
    resolution: ResolutionPreset,
    z_min: f32,
    z_max: f32,
    has_finite_height: bool,
}

impl SurfaceGrid {
    const EMPTY: SurfaceGrid = SurfaceGrid {
        // Startup always resamples this cache before rendering. Keeping its
        // dormant contents zero makes the persistent maximum-capacity storage
        // live in `.bss` rather than copying a 5 KiB initializer from flash.
        heights: [[0.0; MAX_COLUMNS]; MAX_ROWS],
        sample_x: [0.0; MAX_COLUMNS],
        sample_y: [0.0; MAX_ROWS],
        triangle_shades: [[[0; TRIANGLES_PER_CELL]; MAX_COLUMNS - 1]; MAX_ROWS - 1],
        resolution: ResolutionPreset::Low,
        z_min: 0.0,
        z_max: 0.0,
        has_finite_height: false,
    };
}

// The sampled grid is long-lived application state, not a render-local array.
// It is private to this module and accessed only by the cooperative firmware
// entry loop through `with_active_surface`; no interrupt or nested render path
// can observe or mutate it concurrently.
static mut ACTIVE_SURFACE: SurfaceGrid = SurfaceGrid::EMPTY;

/// Accesses the application's one persistent sampled surface cache.
///
/// SAFETY: the NumWorks app has one non-reentrant main loop. Callers complete
/// sampling before reborrowing the grid immutably for rendering, and no Rust
/// interrupt handler accesses this module-private static.
pub fn with_active_surface<R>(callback: impl FnOnce(&mut SurfaceGrid) -> R) -> R {
    unsafe { callback(&mut *core::ptr::addr_of_mut!(ACTIVE_SURFACE)) }
}

impl SurfaceGrid {
    /// Evaluates a function once at every regular-grid sample.
    #[cfg(test)]
    pub fn sample<F: SurfaceFunction>(domain: Domain, function: &F) -> SurfaceGrid {
        Self::sample_with_resolution(domain, function, ResolutionPreset::Standard)
    }

    /// Evaluates a function using one explicitly selected fixed grid.
    #[cfg(test)]
    pub fn sample_with_resolution<F: SurfaceFunction>(
        domain: Domain,
        function: &F,
        resolution: ResolutionPreset,
    ) -> SurfaceGrid {
        let mut grid = SurfaceGrid {
            heights: [[f32::NAN; MAX_COLUMNS]; MAX_ROWS],
            sample_x: [f32::NAN; MAX_COLUMNS],
            sample_y: [f32::NAN; MAX_ROWS],
            triangle_shades: [[[0; TRIANGLES_PER_CELL]; MAX_COLUMNS - 1]; MAX_ROWS - 1],
            resolution,
            z_min: 0.0,
            z_max: 0.0,
            has_finite_height: false,
        };
        grid.resample_with_resolution(domain, function, resolution);
        grid
    }

    /// Replaces all cached heights after the expression or domain changes.
    #[cfg(test)]
    pub fn resample<F: SurfaceFunction>(&mut self, domain: Domain, function: &F) {
        self.resample_with_resolution(domain, function, ResolutionPreset::Standard);
    }

    /// Replaces cached samples using the selected fixed grid.
    pub fn resample_with_resolution<F: SurfaceFunction>(
        &mut self,
        domain: Domain,
        function: &F,
        resolution: ResolutionPreset,
    ) {
        self.resolution = resolution;
        self.z_min = 0.0;
        self.z_max = 0.0;
        self.has_finite_height = false;

        let columns = self.columns();
        let rows = self.rows();
        self.triangle_shades = [[[0; TRIANGLES_PER_CELL]; MAX_COLUMNS - 1]; MAX_ROWS - 1];
        let mut column = 0;
        while column < columns {
            self.sample_x[column] = domain.sample_x(column, columns);
            column += 1;
        }
        let mut row = 0;
        while row < rows {
            self.sample_y[row] = domain.sample_y(row, rows);
            row += 1;
        }

        row = 0;
        while row < rows {
            let y = self.sample_y[row];
            column = 0;
            while column < columns {
                let x = self.sample_x[column];
                let z = function.evaluate(x, y);
                self.heights[row][column] = z;
                if z.is_finite() {
                    if !self.has_finite_height {
                        self.z_min = z;
                        self.z_max = z;
                        self.has_finite_height = true;
                    } else {
                        if z < self.z_min {
                            self.z_min = z;
                        }
                        if z > self.z_max {
                            self.z_max = z;
                        }
                    }
                }
                column += 1;
            }
            row += 1;
        }
        self.rebuild_triangle_shades(domain);
    }

    /// Reconstructs one world point without reevaluating the expression.
    /// Out-of-range indices return a non-finite point rather than panicking.
    pub fn point(&self, domain: Domain, column: usize, row: usize) -> Point3 {
        if row >= self.rows() || column >= self.columns() {
            return Point3 {
                x: f32::NAN,
                y: f32::NAN,
                z: f32::NAN,
            };
        }
        Point3 {
            x: domain.sample_x(column, self.columns()),
            y: domain.sample_y(row, self.rows()),
            z: self.heights[row][column],
        }
    }

    /// Returns one cached-coordinate point for Solid projection only.
    ///
    /// The cached coordinates are exactly those used for the most recent
    /// sampling pass. Keeping this separate from [`Self::point`] protects the
    /// released Wireframe call path from Solid-specific optimization changes.
    pub fn solid_point(&self, column: usize, row: usize) -> Point3 {
        if row >= self.rows() || column >= self.columns() {
            return Point3 {
                x: f32::NAN,
                y: f32::NAN,
                z: f32::NAN,
            };
        }
        Point3 {
            x: self.sample_x[column],
            y: self.sample_y[row],
            z: self.heights[row][column],
        }
    }

    /// Returns the cached Solid triangle validity/diffuse-light values.
    ///
    /// This is rebuilt transactionally at the end of each surface resample, so
    /// camera-only redraws do not recompute up to 1,536 normals, square roots,
    /// or divisions. Wireframe never reads this cache.
    pub fn triangle_shades(&self) -> &TriangleShades {
        &self.triangle_shades
    }

    /// Active sampling choice represented by this cache.
    #[cfg(test)]
    pub const fn resolution(&self) -> ResolutionPreset {
        self.resolution
    }

    /// Active world-X sample count.
    pub const fn columns(&self) -> usize {
        self.resolution.columns()
    }

    /// Active world-Y sample count.
    pub const fn rows(&self) -> usize {
        self.resolution.rows()
    }

    /// Cached finite height range and whether at least one finite sample exists.
    pub fn z_range(&self) -> (f32, f32, bool) {
        (self.z_min, self.z_max, self.has_finite_height)
    }

    /// Rebuilds Solid-mode Lambert diffuse light and triangle validity after sampling.
    ///
    /// Triangle zero is `(top-left, top-right, bottom-right)` and triangle one
    /// is `(top-left, bottom-right, bottom-left)`. Both wind toward world `+z`.
    fn rebuild_triangle_shades(&mut self, domain: Domain) {
        let maximum_height_jump =
            discontinuity_limit(domain, self.z_min, self.z_max, self.has_finite_height);
        let mut row = 0;
        while row + 1 < self.rows() {
            let mut column = 0;
            while column + 1 < self.columns() {
                let top_left = self.point(domain, column, row);
                let top_right = self.point(domain, column + 1, row);
                let bottom_left = self.point(domain, column, row + 1);
                let bottom_right = self.point(domain, column + 1, row + 1);
                // A finite pole can fall between samples, so checking only the
                // four values is insufficient. Across 1/x- or tan-like poles the
                // edge delta reverses the trend on both sides and is markedly
                // larger than a neighboring delta. Reject the complete regular
                // cell in that case; this leaves a conservative gap without
                // changing the released wireframe path or evaluating midpoints.
                if self.cell_reverses_sample_trend(row, column) {
                    self.triangle_shades[row][column] = [0; TRIANGLES_PER_CELL];
                } else {
                    self.triangle_shades[row][column][0] =
                        triangle_light(top_left, top_right, bottom_right, maximum_height_jump);
                    self.triangle_shades[row][column][1] =
                        triangle_light(top_left, bottom_right, bottom_left, maximum_height_jump);
                }
                column += 1;
            }
            row += 1;
        }
    }

    #[cfg(test)]
    fn build_triangle_shades_reference(&self, domain: Domain, shades: &mut TriangleShades) {
        let maximum_height_jump =
            discontinuity_limit(domain, self.z_min, self.z_max, self.has_finite_height);
        let mut row = 0;
        while row + 1 < self.rows() {
            let mut column = 0;
            while column + 1 < self.columns() {
                let top_left = self.point(domain, column, row);
                let top_right = self.point(domain, column + 1, row);
                let bottom_left = self.point(domain, column, row + 1);
                let bottom_right = self.point(domain, column + 1, row + 1);
                if self.cell_reverses_sample_trend(row, column) {
                    shades[row][column] = [0; TRIANGLES_PER_CELL];
                } else {
                    shades[row][column][0] =
                        triangle_light(top_left, top_right, bottom_right, maximum_height_jump);
                    shades[row][column][1] =
                        triangle_light(top_left, bottom_right, bottom_left, maximum_height_jump);
                }
                column += 1;
            }
            row += 1;
        }
    }

    fn cell_reverses_sample_trend(&self, row: usize, column: usize) -> bool {
        horizontal_edge_reverses_trend(&self.heights, self.rows(), self.columns(), row, column)
            || horizontal_edge_reverses_trend(
                &self.heights,
                self.rows(),
                self.columns(),
                row + 1,
                column,
            )
            || vertical_edge_reverses_trend(&self.heights, self.rows(), self.columns(), row, column)
            || vertical_edge_reverses_trend(
                &self.heights,
                self.rows(),
                self.columns(),
                row,
                column + 1,
            )
    }
}

fn horizontal_edge_reverses_trend(
    heights: &[[f32; MAX_COLUMNS]; MAX_ROWS],
    rows: usize,
    columns: usize,
    row: usize,
    column: usize,
) -> bool {
    if row >= rows || column + 1 >= columns {
        return false;
    }
    let current = heights[row][column + 1] - heights[row][column];
    let previous = if column > 0 {
        Some(heights[row][column] - heights[row][column - 1])
    } else {
        None
    };
    let next = if column + 2 < columns {
        Some(heights[row][column + 2] - heights[row][column + 1])
    } else {
        None
    };
    delta_reverses_trend(current, previous, next)
}

fn vertical_edge_reverses_trend(
    heights: &[[f32; MAX_COLUMNS]; MAX_ROWS],
    rows: usize,
    columns: usize,
    row: usize,
    column: usize,
) -> bool {
    if row + 1 >= rows || column >= columns {
        return false;
    }
    let current = heights[row + 1][column] - heights[row][column];
    let previous = if row > 0 {
        Some(heights[row][column] - heights[row - 1][column])
    } else {
        None
    };
    let next = if row + 2 < rows {
        Some(heights[row + 2][column] - heights[row + 1][column])
    } else {
        None
    };
    delta_reverses_trend(current, previous, next)
}

fn delta_reverses_trend(current: f32, previous: Option<f32>, next: Option<f32>) -> bool {
    if !current.is_finite() {
        return false;
    }
    let previous = previous.filter(|value| value.is_finite());
    let next = next.filter(|value| value.is_finite());
    let current_magnitude = current.abs();
    match (previous, next) {
        (Some(previous), Some(next)) => {
            opposite_sign(current, previous)
                && opposite_sign(current, next)
                && current_magnitude > previous.abs().min(next.abs()) * 1.5
        }
        (Some(neighbor), None) | (None, Some(neighbor)) => {
            opposite_sign(current, neighbor) && current_magnitude > neighbor.abs() * 1.5
        }
        (None, None) => false,
    }
}

fn opposite_sign(first: f32, second: f32) -> bool {
    (first < 0.0 && second > 0.0) || (first > 0.0 && second < 0.0)
}

/// Quantizes one fixed-world-light Lambert term to 1..=255. Zero is kept as an
/// invalid sentinel so the renderer needs no parallel triangle-validity bitmap.
fn triangle_light(a: Point3, b: Point3, c: Point3, maximum_height_jump: f32) -> u8 {
    if !point_is_finite(a) || !point_is_finite(b) || !point_is_finite(c) {
        return 0;
    }
    // A mathematically finite sample can still sit arbitrarily close to a pole.
    // Refusing an implausibly tall local edge prevents a filled triangle from
    // bridging that discontinuity or producing a screen-sized spike.
    if (a.z - b.z).abs() > maximum_height_jump
        || (b.z - c.z).abs() > maximum_height_jump
        || (c.z - a.z).abs() > maximum_height_jump
    {
        return 0;
    }

    let ab_x = b.x - a.x;
    let ab_y = b.y - a.y;
    let ab_z = b.z - a.z;
    let ac_x = c.x - a.x;
    let ac_y = c.y - a.y;
    let ac_z = c.z - a.z;
    let normal_x = ab_y * ac_z - ab_z * ac_y;
    let normal_y = ab_z * ac_x - ab_x * ac_z;
    let normal_z = ab_x * ac_y - ab_y * ac_x;
    let length_squared = normal_x * normal_x + normal_y * normal_y + normal_z * normal_z;
    if !length_squared.is_finite() || length_squared <= 0.00000001 {
        return 0;
    }
    let length = math::sqrt(length_squared);
    if !length.is_finite() || length <= 0.0 {
        return 0;
    }

    // The direction is approximately normalized and intentionally fixed in
    // world space, making lighting stable while the user orbits the camera.
    const LIGHT_X: f32 = -0.34;
    const LIGHT_Y: f32 = -0.44;
    const LIGHT_Z: f32 = 0.83;
    let diffuse = ((normal_x * LIGHT_X + normal_y * LIGHT_Y + normal_z * LIGHT_Z) / length)
        .max(0.0)
        .min(1.0);
    // Zero is reserved for invalid triangles. The remaining range stores only
    // normalized diffuse light, keeping the cache independent of the selected
    // ambient/diffuse preset and RGB565 surface color.
    1_u8.saturating_add((diffuse * 254.0) as u8)
}

fn point_is_finite(point: Point3) -> bool {
    point.x.is_finite() && point.y.is_finite() && point.z.is_finite()
}

/// Chooses a deliberately generous local-jump threshold. The valid sampled
/// range contributes for naturally tall surfaces, but is capped relative to the
/// XY domain so one pole-sized outlier cannot make the test self-defeating.
fn discontinuity_limit(domain: Domain, z_min: f32, z_max: f32, has_height: bool) -> f32 {
    let x_span = (domain.x_max - domain.x_min).abs();
    let y_span = (domain.y_max - domain.y_min).abs();
    let domain_scale = x_span.max(y_span).max(1.0);
    let base_limit = domain_scale * 16.0;
    if !has_height || !z_min.is_finite() || !z_max.is_finite() {
        return base_limit;
    }
    let capped_range = (z_max - z_min).abs().min(domain_scale * 64.0);
    base_limit.max(capped_range * 0.75)
}

/// Evaluates one uncached point for host-side sampling tests. Frame rendering
/// always uses `SurfaceGrid::point` instead.
#[cfg(test)]
pub fn point<F: SurfaceFunction>(
    domain: Domain,
    column: usize,
    row: usize,
    function: &F,
) -> Point3 {
    let x = domain.sample_x(column, COLUMNS);
    let y = domain.sample_y(row, ROWS);
    Point3 {
        x,
        y,
        z: function.evaluate(x, y),
    }
}

fn sample(min: f32, max: f32, index: usize, count: usize) -> f32 {
    if count <= 1 {
        return min;
    }
    min + (max - min) * index as f32 / (count - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::CompiledExpression;
    use core::cell::Cell;

    fn triangle_shades(grid: &SurfaceGrid, _domain: Domain) -> TriangleShades {
        *grid.triangle_shades()
    }

    fn shade_counts(shades: &TriangleShades) -> (usize, usize) {
        let mut valid = 0;
        let mut invalid = 0;
        let mut row = 0;
        while row + 1 < ROWS {
            let mut column = 0;
            while column + 1 < COLUMNS {
                let mut triangle = 0;
                while triangle < TRIANGLES_PER_CELL {
                    if shades[row][column][triangle] == 0 {
                        invalid += 1;
                    } else {
                        valid += 1;
                    }
                    triangle += 1;
                }
                column += 1;
            }
            row += 1;
        }
        (valid, invalid)
    }

    struct Plane;

    impl SurfaceFunction for Plane {
        fn evaluate(&self, x: f32, y: f32) -> f32 {
            x + y
        }
    }

    #[test]
    fn solid_coordinate_and_lighting_caches_match_the_sampling_reference() {
        let domain = Domain::new(-2.5, 3.75, -4.0, 1.5);
        let grid = SurfaceGrid::sample(domain, &Plane);
        let mut row = 0;
        while row < ROWS {
            let mut column = 0;
            while column < COLUMNS {
                let wireframe_point = grid.point(domain, column, row);
                let solid_point = grid.solid_point(column, row);
                assert_eq!(solid_point.x, wireframe_point.x);
                assert_eq!(solid_point.y, wireframe_point.y);
                assert_eq!(solid_point.z, wireframe_point.z);
                column += 1;
            }
            row += 1;
        }

        let mut reference: TriangleShades =
            [[[0; TRIANGLES_PER_CELL]; MAX_COLUMNS - 1]; MAX_ROWS - 1];
        grid.build_triangle_shades_reference(domain, &mut reference);
        assert_eq!(*grid.triangle_shades(), reference);
    }

    #[test]
    fn domain_sampling_reaches_each_boundary() {
        let domain = Domain::new(-2.0, 6.0, -4.0, 8.0);
        assert_eq!(domain.sample_x(0, 5), -2.0);
        assert_eq!(domain.sample_x(4, 5), 6.0);
        assert_eq!(domain.sample_y(0, 3), -4.0);
        assert_eq!(domain.sample_y(2, 3), 8.0);
    }

    #[test]
    fn resolution_presets_have_exact_bounded_topology() {
        assert_eq!(
            (
                ResolutionPreset::Low.columns(),
                ResolutionPreset::Low.rows()
            ),
            (17, 13)
        );
        assert_eq!(ResolutionPreset::Low.triangle_count(), 384);
        assert_eq!(
            (
                ResolutionPreset::Standard.columns(),
                ResolutionPreset::Standard.rows()
            ),
            (25, 19)
        );
        assert_eq!(ResolutionPreset::Standard.triangle_count(), 864);
        assert_eq!(
            (
                ResolutionPreset::High.columns(),
                ResolutionPreset::High.rows()
            ),
            (33, 25)
        );
        assert_eq!(ResolutionPreset::High.triangle_count(), 1_536);
        assert!(ResolutionPreset::High.columns() <= MAX_COLUMNS);
        assert!(ResolutionPreset::High.rows() <= MAX_ROWS);
    }

    #[test]
    fn every_resolution_samples_inclusive_monotonic_domain_endpoints() {
        let domain = Domain::new(-2.5, 3.75, -4.0, 1.5);
        for resolution in [
            ResolutionPreset::Low,
            ResolutionPreset::Standard,
            ResolutionPreset::High,
        ] {
            let grid = SurfaceGrid::sample_with_resolution(domain, &Plane, resolution);
            assert_eq!(grid.solid_point(0, 0).x, domain.x_min);
            assert_eq!(grid.solid_point(0, 0).y, domain.y_min);
            assert_eq!(
                grid.solid_point(grid.columns() - 1, grid.rows() - 1).x,
                domain.x_max
            );
            assert_eq!(
                grid.solid_point(grid.columns() - 1, grid.rows() - 1).y,
                domain.y_max
            );
            let mut column = 1;
            while column < grid.columns() {
                assert!(grid.solid_point(column - 1, 0).x < grid.solid_point(column, 0).x);
                column += 1;
            }
        }
    }

    #[test]
    fn standard_coordinates_match_the_released_sampling_formula_bit_for_bit() {
        let domain = Domain::DEFAULT;
        let grid = SurfaceGrid::sample_with_resolution(domain, &Plane, ResolutionPreset::Standard);
        let mut column = 0;
        while column < COLUMNS {
            let reference =
                domain.x_min + (domain.x_max - domain.x_min) * column as f32 / (COLUMNS - 1) as f32;
            assert_eq!(grid.solid_point(column, 0).x.to_bits(), reference.to_bits());
            column += 1;
        }
        let mut row = 0;
        while row < ROWS {
            let reference =
                domain.y_min + (domain.y_max - domain.y_min) * row as f32 / (ROWS - 1) as f32;
            assert_eq!(grid.solid_point(0, row).y.to_bits(), reference.to_bits());
            row += 1;
        }
    }

    #[test]
    fn active_triangle_range_matches_each_resolution_and_ignores_inactive_capacity() {
        let expression = CompiledExpression::compile("sin(x) * cos(y)").expect("expression");
        for resolution in [
            ResolutionPreset::Low,
            ResolutionPreset::Standard,
            ResolutionPreset::High,
        ] {
            let grid =
                SurfaceGrid::sample_with_resolution(Domain::DEFAULT, &expression, resolution);
            let mut active = 0;
            let mut row = 0;
            while row + 1 < grid.rows() {
                let mut column = 0;
                while column + 1 < grid.columns() {
                    active += 2;
                    column += 1;
                }
                row += 1;
            }
            assert_eq!(active, resolution.triangle_count());
            assert!(!grid.solid_point(grid.columns(), 0).z.is_finite());
            assert!(!grid.solid_point(0, grid.rows()).z.is_finite());
        }
    }

    #[test]
    fn surface_points_use_the_configured_domain() {
        let domain = Domain::new(1.0, 5.0, 2.0, 6.0);
        let first = point(domain, 0, 0, &Plane);
        let last = point(domain, COLUMNS - 1, ROWS - 1, &Plane);
        assert_eq!((first.x, first.y, first.z), (1.0, 2.0, 3.0));
        assert_eq!((last.x, last.y, last.z), (5.0, 6.0, 11.0));
    }

    #[test]
    fn cached_grid_samples_once_and_reuses_heights() {
        struct Counted<'a>(&'a Cell<usize>);
        impl SurfaceFunction for Counted<'_> {
            fn evaluate(&self, x: f32, y: f32) -> f32 {
                self.0.set(self.0.get() + 1);
                x - y
            }
        }

        let calls = Cell::new(0);
        let function = Counted(&calls);
        let grid = SurfaceGrid::sample(Domain::DEFAULT, &function);
        assert_eq!(calls.get(), COLUMNS * ROWS);
        let _ = grid.point(Domain::DEFAULT, 0, 0);
        let _ = grid.point(Domain::DEFAULT, COLUMNS - 1, ROWS - 1);
        assert_eq!(calls.get(), COLUMNS * ROWS);
    }

    #[test]
    fn cached_grid_reports_finite_range() {
        let grid = SurfaceGrid::sample(Domain::new(-1.0, 1.0, -2.0, 2.0), &Plane);
        let (minimum, maximum, finite) = grid.z_range();
        assert!(finite);
        assert!((minimum + 3.0).abs() < 0.0001);
        assert!((maximum - 3.0).abs() < 0.0001);
    }

    #[test]
    fn flat_surface_has_valid_cached_triangle_lighting() {
        struct Flat;
        impl SurfaceFunction for Flat {
            fn evaluate(&self, _x: f32, _y: f32) -> f32 {
                0.0
            }
        }
        let grid = SurfaceGrid::sample(Domain::DEFAULT, &Flat);
        let shades = triangle_shades(&grid, Domain::DEFAULT);
        assert_ne!(shades[0][0][0], 0);
        assert_ne!(shades[ROWS - 2][COLUMNS - 2][1], 0);
        assert_eq!(core::mem::size_of::<TriangleShades>(), 1_536);
        // 1,900 height bytes plus 864 cached shades and 176 cached X/Y bytes,
        // with the range metadata/alignment required by the target ABI.
        assert_eq!(core::mem::size_of::<SurfaceGrid>(), 5_080);
    }

    #[test]
    fn cached_diffuse_light_is_deterministic_and_bounded() {
        let a = Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let b = Point3 {
            x: 1.0,
            y: 0.0,
            z: 0.25,
        };
        let c = Point3 {
            x: 1.0,
            y: 1.0,
            z: 0.5,
        };
        let first = triangle_light(a, b, c, 100.0);
        let second = triangle_light(a, b, c, 100.0);
        assert_eq!(first, second);
        assert!((1..=255).contains(&first));
        assert_eq!(triangle_light(a, b, Point3 { z: f32::NAN, ..c }, 100.0), 0);
    }

    #[test]
    fn triangles_touching_non_finite_samples_are_invalid() {
        struct InvalidAtZero;
        impl SurfaceFunction for InvalidAtZero {
            fn evaluate(&self, x: f32, _y: f32) -> f32 {
                if x.abs() < 0.001 {
                    f32::NAN
                } else {
                    0.0
                }
            }
        }
        let grid = SurfaceGrid::sample(Domain::DEFAULT, &InvalidAtZero);
        let shades = triangle_shades(&grid, Domain::DEFAULT);
        assert_eq!(shades[0][COLUMNS / 2 - 1][0], 0);
        assert_eq!(shades[0][COLUMNS / 2][1], 0);
        assert_ne!(shades[0][0][0], 0);
    }

    #[test]
    fn pole_sized_finite_jump_is_rejected_but_ordinary_steep_plane_survives() {
        struct FinitePole;
        impl SurfaceFunction for FinitePole {
            fn evaluate(&self, x: f32, _y: f32) -> f32 {
                if x.abs() < 0.001 {
                    1.0e20
                } else {
                    0.0
                }
            }
        }
        struct SteepPlane;
        impl SurfaceFunction for SteepPlane {
            fn evaluate(&self, x: f32, _y: f32) -> f32 {
                1_000.0 * x
            }
        }

        let pole = SurfaceGrid::sample(Domain::DEFAULT, &FinitePole);
        let pole_shades = triangle_shades(&pole, Domain::DEFAULT);
        assert_eq!(pole_shades[0][COLUMNS / 2 - 1][0], 0);
        assert_ne!(pole_shades[0][0][0], 0);

        let steep = SurfaceGrid::sample(Domain::DEFAULT, &SteepPlane);
        let steep_shades = triangle_shades(&steep, Domain::DEFAULT);
        assert_ne!(steep_shades[0][COLUMNS / 2 - 1][0], 0);
        assert_ne!(steep_shades[0][COLUMNS / 2][1], 0);
    }

    #[test]
    fn trend_reversal_rejects_both_triangles_in_the_crossing_cell() {
        let row = ROWS / 2;
        let column = COLUMNS / 2;
        let mut heights = [[0.0_f32; MAX_COLUMNS]; MAX_ROWS];
        let mut y = 0;
        while y < ROWS {
            // The central edge climbs sharply, while both adjacent edges fall.
            // This is the finite-sample signature used for a pole between grid
            // points; it must open the complete cell rather than leave half a
            // triangle bridging the discontinuity.
            heights[y][column] = -1.0;
            heights[y][column + 1] = 10.0;
            heights[y][column + 2] = 9.0;
            y += 1;
        }
        let mut grid = SurfaceGrid {
            heights,
            sample_x: [f32::NAN; MAX_COLUMNS],
            sample_y: [f32::NAN; MAX_ROWS],
            triangle_shades: [[[0; TRIANGLES_PER_CELL]; MAX_COLUMNS - 1]; MAX_ROWS - 1],
            resolution: ResolutionPreset::Standard,
            z_min: -1.0,
            z_max: 10.0,
            has_finite_height: true,
        };

        assert!(horizontal_edge_reverses_trend(
            &grid.heights,
            grid.rows(),
            grid.columns(),
            row,
            column,
        ));
        assert!(grid.cell_reverses_sample_trend(row, column));

        grid.rebuild_triangle_shades(Domain::DEFAULT);
        let shades = triangle_shades(&grid, Domain::DEFAULT);
        assert_eq!(shades[row][column], [0, 0]);
        assert_ne!(shades[row][column - 1], [0, 0]);
        assert_ne!(shades[row][column + 1], [0, 0]);
    }

    #[test]
    fn reciprocal_pole_invalidates_every_triangle_in_adjacent_cells() {
        let reciprocal = CompiledExpression::compile("1/x").expect("reciprocal expression");
        let grid = SurfaceGrid::sample(Domain::DEFAULT, &reciprocal);
        let shades = triangle_shades(&grid, Domain::DEFAULT);
        let zero_column = COLUMNS / 2;

        let mut row = 0;
        while row + 1 < ROWS {
            assert_eq!(shades[row][zero_column - 1], [0, 0]);
            assert_eq!(shades[row][zero_column], [0, 0]);
            row += 1;
        }
        assert_ne!(shades[0][0], [0, 0]);
    }

    #[test]
    fn required_hardware_matrix_samples_without_bridging_invalid_regions() {
        let smooth = [
            "sin(x) * cos(y)",
            "x^2 + y^2",
            "x^2 - y^2",
            "sin(sqrt(x^2 + y^2))",
        ];
        for source in smooth {
            let expression = CompiledExpression::compile(source).expect("required expression");
            let grid = SurfaceGrid::sample(Domain::DEFAULT, &expression);
            let (valid, _) = shade_counts(&triangle_shades(&grid, Domain::DEFAULT));
            assert!(valid > 0, "{}", source);
        }

        let discontinuous = ["sqrt(x)", "1/x", "tan(x)", "1/(x*y)"];
        for source in discontinuous {
            let expression = CompiledExpression::compile(source).expect("required expression");
            let grid = SurfaceGrid::sample(Domain::DEFAULT, &expression);
            let (valid, invalid) = shade_counts(&triangle_shades(&grid, Domain::DEFAULT));
            assert!(valid > 0, "{}", source);
            assert!(invalid > 0, "{}", source);
        }
    }

    #[test]
    fn new_function_domain_failures_remain_invalid_surface_samples() {
        for source in ["ln(x)", "log(10,x)", "log(1,x)", "asin(x)", "acos(x)"] {
            let expression = CompiledExpression::compile(source).expect("valid expression");
            let grid = SurfaceGrid::sample(Domain::DEFAULT, &expression);
            let mut saw_invalid_height = false;
            let mut row = 0;
            while row < ROWS {
                let mut column = 0;
                while column < COLUMNS {
                    if !grid.solid_point(column, row).z.is_finite() {
                        saw_invalid_height = true;
                    }
                    column += 1;
                }
                row += 1;
            }
            let (_, invalid) = shade_counts(&grid.triangle_shades());
            assert!(saw_invalid_height, "{}", source);
            assert!(invalid > 0, "{}", source);
        }
    }

    #[test]
    fn domain_validation_rejects_unsafe_bounds() {
        assert_eq!(Domain::DEFAULT.validate(), Ok(()));
        assert_eq!(
            Domain::new(f32::NAN, 1.0, -1.0, 1.0).validate(),
            Err(DomainError::NonFinite)
        );
        assert_eq!(
            Domain::new(2.0, 1.0, -1.0, 1.0).validate(),
            Err(DomainError::Inverted)
        );
        assert_eq!(
            Domain::new(0.0, 0.001, -1.0, 1.0).validate(),
            Err(DomainError::TooNarrow)
        );
        assert_eq!(
            Domain::new(-2_000.0, 2_000.0, -1.0, 1.0).validate(),
            Err(DomainError::TooLarge)
        );
    }
}
