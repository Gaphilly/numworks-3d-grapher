//! Mathematical domain and fixed-resolution surface sampling.
//!
//! The renderer uses a regular 25×19 grid. Only the 475 `f32` heights are cached;
//! `x` and `y` are recovered from the domain, avoiding redundant RAM. Solid-only
//! triangle lighting is built into a caller-owned transient array so selecting
//! wireframe retains the original surface-cache and render-stack footprint.
//! Equation bytecode is evaluated only after an expression/domain change, never
//! for a camera-only redraw.

use crate::function::SurfaceFunction;
use crate::math;

/// Number of samples along world `x`.
pub const COLUMNS: usize = 25;
/// Number of samples along world `y`.
pub const ROWS: usize = 19;
/// Two consistently wound triangles cover every regular-grid cell.
pub const TRIANGLES_PER_CELL: usize = 2;
/// Transient one-byte light/validity values for every regular-grid triangle.
/// Zero means invalid; values 1..=255 are quantized brightness.
pub type TriangleShades = [[[u8; TRIANGLES_PER_CELL]; COLUMNS - 1]; ROWS - 1];
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
/// Heights occupy exactly 1,900 bytes plus small range metadata. NaN/infinite
/// evaluations remain in this cache and later invalidate every solid triangle
/// that touches them, so ordinary invalid mathematical input cannot reach
/// triangle rasterization or bridge a discontinuity.
pub struct SurfaceGrid {
    heights: [[f32; COLUMNS]; ROWS],
    z_min: f32,
    z_max: f32,
    has_finite_height: bool,
}

impl SurfaceGrid {
    /// Evaluates a function once at every regular-grid sample.
    pub fn sample<F: SurfaceFunction>(domain: Domain, function: &F) -> SurfaceGrid {
        let mut grid = SurfaceGrid {
            heights: [[f32::NAN; COLUMNS]; ROWS],
            z_min: 0.0,
            z_max: 0.0,
            has_finite_height: false,
        };
        grid.resample(domain, function);
        grid
    }

    /// Replaces all cached heights after the expression or domain changes.
    pub fn resample<F: SurfaceFunction>(&mut self, domain: Domain, function: &F) {
        self.z_min = 0.0;
        self.z_max = 0.0;
        self.has_finite_height = false;

        let mut row = 0;
        while row < ROWS {
            let y = domain.sample_y(row, ROWS);
            let mut column = 0;
            while column < COLUMNS {
                let x = domain.sample_x(column, COLUMNS);
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
    }

    /// Reconstructs one world point without reevaluating the expression.
    /// Out-of-range indices return a non-finite point rather than panicking.
    pub fn point(&self, domain: Domain, column: usize, row: usize) -> Point3 {
        if row >= ROWS || column >= COLUMNS {
            return Point3 {
                x: f32::NAN,
                y: f32::NAN,
                z: f32::NAN,
            };
        }
        Point3 {
            x: domain.sample_x(column, COLUMNS),
            y: domain.sample_y(row, ROWS),
            z: self.heights[row][column],
        }
    }

    /// Cached finite height range and whether at least one finite sample exists.
    pub fn z_range(&self) -> (f32, f32, bool) {
        (self.z_min, self.z_max, self.has_finite_height)
    }

    /// Builds solid-mode Lambert lighting and triangle validity into transient
    /// caller-owned storage. Wireframe never calls this function or reserves the
    /// 864-byte array on its render stack.
    ///
    /// Triangle zero is `(top-left, top-right, bottom-right)` and triangle one
    /// is `(top-left, bottom-right, bottom-left)`. Both wind toward world `+z`.
    pub fn build_triangle_shades(&self, domain: Domain, shades: &mut TriangleShades) {
        let maximum_height_jump =
            discontinuity_limit(domain, self.z_min, self.z_max, self.has_finite_height);
        let mut row = 0;
        while row + 1 < ROWS {
            let mut column = 0;
            while column + 1 < COLUMNS {
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
        horizontal_edge_reverses_trend(&self.heights, row, column)
            || horizontal_edge_reverses_trend(&self.heights, row + 1, column)
            || vertical_edge_reverses_trend(&self.heights, row, column)
            || vertical_edge_reverses_trend(&self.heights, row, column + 1)
    }
}

fn horizontal_edge_reverses_trend(
    heights: &[[f32; COLUMNS]; ROWS],
    row: usize,
    column: usize,
) -> bool {
    if row >= ROWS || column + 1 >= COLUMNS {
        return false;
    }
    let current = heights[row][column + 1] - heights[row][column];
    let previous = if column > 0 {
        Some(heights[row][column] - heights[row][column - 1])
    } else {
        None
    };
    let next = if column + 2 < COLUMNS {
        Some(heights[row][column + 2] - heights[row][column + 1])
    } else {
        None
    };
    delta_reverses_trend(current, previous, next)
}

fn vertical_edge_reverses_trend(
    heights: &[[f32; COLUMNS]; ROWS],
    row: usize,
    column: usize,
) -> bool {
    if row + 1 >= ROWS || column >= COLUMNS {
        return false;
    }
    let current = heights[row + 1][column] - heights[row][column];
    let previous = if row > 0 {
        Some(heights[row][column] - heights[row - 1][column])
    } else {
        None
    };
    let next = if row + 2 < ROWS {
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
    let brightness = 0.32 + 0.68 * diffuse;
    let quantized = (brightness * 255.0) as u8;
    if quantized == 0 {
        1
    } else {
        quantized
    }
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

    fn triangle_shades(grid: &SurfaceGrid, domain: Domain) -> TriangleShades {
        let mut shades = [[[0; TRIANGLES_PER_CELL]; COLUMNS - 1]; ROWS - 1];
        grid.build_triangle_shades(domain, &mut shades);
        shades
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
    fn domain_sampling_reaches_each_boundary() {
        let domain = Domain::new(-2.0, 6.0, -4.0, 8.0);
        assert_eq!(domain.sample_x(0, 5), -2.0);
        assert_eq!(domain.sample_x(4, 5), 6.0);
        assert_eq!(domain.sample_y(0, 3), -4.0);
        assert_eq!(domain.sample_y(2, 3), 8.0);
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
        assert_eq!(core::mem::size_of::<TriangleShades>(), 864);
        // Keeping lighting transient preserves the original wireframe cache:
        // 1,900 height bytes, two f32 bounds, one bool, and alignment padding.
        assert_eq!(core::mem::size_of::<SurfaceGrid>(), 1_912);
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
        let mut heights = [[0.0_f32; COLUMNS]; ROWS];
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
        let grid = SurfaceGrid {
            heights,
            z_min: -1.0,
            z_max: 10.0,
            has_finite_height: true,
        };

        assert!(horizontal_edge_reverses_trend(&grid.heights, row, column));
        assert!(grid.cell_reverses_sample_trend(row, column));

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
