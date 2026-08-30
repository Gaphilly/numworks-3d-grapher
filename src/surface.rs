//! Mathematical domain and fixed-resolution surface sampling.
//!
//! The renderer uses a regular 25×19 grid. Only the 475 `f32` heights are
//! cached: `x` and `y` are recovered from the domain, avoiding redundant RAM.
//! This cache is an important performance boundary—equation bytecode is evaluated
//! only after a successful edit or domain change, never for camera-only redraws.

use crate::function::SurfaceFunction;

/// Number of samples along world `x`.
pub const COLUMNS: usize = 25;
/// Number of samples along world `y`.
pub const ROWS: usize = 19;

/// Inclusive rectangular mathematical domain sampled by the surface and used by
/// axes, grid lines, and ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
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
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Fixed-capacity height cache for the current compiled expression and domain.
///
/// Storage is exactly 475 `f32` values (1,900 bytes) plus a small cached range.
/// NaN/infinite evaluations remain in the cache and are rejected by projection,
/// so ordinary invalid mathematical input cannot reach integer rasterization.
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
    use core::cell::Cell;

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
}
