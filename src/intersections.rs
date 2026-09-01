//! Bounded camera-independent intersections between sampled height fields.
//!
//! Marching each of the two regular-grid triangles detects sign changes in
//! `f-g`. Endpoints store only a cell/triangle/edge and 16-bit interpolation
//! fraction; world coordinates are reconstructed from the shared samples when
//! projected. No symbolic solving, iteration, or allocation is involved.

use crate::functions::{FUNCTION_PAIRS, MAX_FUNCTION_PAIRS};
use crate::surface::{Point3, SurfaceBank};

pub const MAX_INTERSECTION_SEGMENTS_PER_PAIR: usize = 256;
#[allow(dead_code)]
pub const MAX_TOTAL_INTERSECTION_SEGMENTS: usize =
    MAX_FUNCTION_PAIRS * MAX_INTERSECTION_SEGMENTS_PER_PAIR;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(transparent)]
pub struct PackedEndpoint(u32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackedSegment {
    pub first: PackedEndpoint,
    pub second: PackedEndpoint,
}

impl PackedSegment {
    const EMPTY: Self = Self {
        first: PackedEndpoint(0),
        second: PackedEndpoint(0),
    };
}

pub struct PairIntersections {
    segments: [PackedSegment; MAX_INTERSECTION_SEGMENTS_PER_PAIR],
    stored: u16,
    total: u16,
    truncated: bool,
}

impl PairIntersections {
    const EMPTY: Self = Self {
        segments: [PackedSegment::EMPTY; MAX_INTERSECTION_SEGMENTS_PER_PAIR],
        stored: 0,
        total: 0,
        truncated: false,
    };

    pub fn segments(&self) -> &[PackedSegment] {
        &self.segments[..self.stored as usize]
    }

    pub fn total(&self) -> u16 {
        self.total
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

pub struct IntersectionCache {
    pairs: [PairIntersections; MAX_FUNCTION_PAIRS],
    valid_mask: u8,
    visibility_mask: u8,
}

impl IntersectionCache {
    const EMPTY: Self = Self {
        pairs: [
            PairIntersections::EMPTY,
            PairIntersections::EMPTY,
            PairIntersections::EMPTY,
            PairIntersections::EMPTY,
            PairIntersections::EMPTY,
            PairIntersections::EMPTY,
        ],
        valid_mask: 0,
        visibility_mask: 0,
    };

    pub fn initialize(&mut self) {
        self.visibility_mask = (1 << MAX_FUNCTION_PAIRS) - 1;
    }

    pub fn pair(&self, index: usize) -> Option<&PairIntersections> {
        if index < MAX_FUNCTION_PAIRS && self.valid_mask & (1 << index) != 0 {
            Some(&self.pairs[index])
        } else {
            None
        }
    }

    pub fn visibility_mask(&self) -> u8 {
        self.visibility_mask
    }

    pub fn toggle_visibility(&mut self, pair: usize) {
        if pair < MAX_FUNCTION_PAIRS {
            self.visibility_mask ^= 1 << pair;
        }
    }

    pub fn invalidate(&mut self, mask: u8) {
        self.valid_mask &= !mask;
    }

    #[inline(never)]
    pub fn rebuild_pair(&mut self, pair_index: usize, surfaces: &SurfaceBank) {
        if pair_index >= MAX_FUNCTION_PAIRS {
            return;
        }
        let (first, second) = FUNCTION_PAIRS[pair_index];
        let first_surface = match surfaces.surface(first) {
            Some(surface) => surface,
            None => {
                self.pairs[pair_index] = PairIntersections::EMPTY;
                self.valid_mask |= 1 << pair_index;
                return;
            }
        };
        let second_surface = match surfaces.surface(second) {
            Some(surface) => surface,
            None => {
                self.pairs[pair_index] = PairIntersections::EMPTY;
                self.valid_mask |= 1 << pair_index;
                return;
            }
        };

        self.pairs[pair_index] = PairIntersections::EMPTY;
        let output = &mut self.pairs[pair_index];
        let total = scan_pair(
            surfaces,
            first_surface.triangle_shades(),
            second_surface.triangle_shades(),
            first,
            second,
            |candidate, ordinal| {
                if ordinal < MAX_INTERSECTION_SEGMENTS_PER_PAIR {
                    push_unique(output, candidate, surfaces.columns());
                }
            },
        );
        output.total = total.min(u16::MAX as usize) as u16;
        output.truncated = total > MAX_INTERSECTION_SEGMENTS_PER_PAIR;
        if output.truncated {
            output.stored = 0;
            let denominator = MAX_INTERSECTION_SEGMENTS_PER_PAIR - 1;
            let _ = scan_pair(
                surfaces,
                first_surface.triangle_shades(),
                second_surface.triangle_shades(),
                first,
                second,
                |candidate, ordinal| {
                    let mut selected = 0;
                    while selected < MAX_INTERSECTION_SEGMENTS_PER_PAIR {
                        let target = selected * (total - 1) / denominator;
                        if ordinal == target {
                            push_unique(output, candidate, surfaces.columns());
                            break;
                        }
                        if target > ordinal {
                            break;
                        }
                        selected += 1;
                    }
                },
            );
        }
        self.valid_mask |= 1 << pair_index;
    }

    pub fn representative(&self, pair: usize, surfaces: &SurfaceBank) -> Option<Point3> {
        let data = self.pair(pair)?;
        let functions = FUNCTION_PAIRS[pair];
        let center_x = (surfaces.x(0) + surfaces.x(surfaces.columns() - 1)) * 0.5;
        let center_y = (surfaces.y(0) + surfaces.y(surfaces.rows() - 1)) * 0.5;
        let mut result = None;
        let mut best = f32::INFINITY;
        for segment in data.segments() {
            let first = reconstruct_endpoint(segment.first, surfaces, functions.0, functions.1)?;
            let second = reconstruct_endpoint(segment.second, surfaces, functions.0, functions.1)?;
            let midpoint = Point3 {
                x: (first.x + second.x) * 0.5,
                y: (first.y + second.y) * 0.5,
                z: (first.z + second.z) * 0.5,
            };
            let dx = midpoint.x - center_x;
            let dy = midpoint.y - center_y;
            let distance = dx * dx + dy * dy;
            if distance < best {
                best = distance;
                result = Some(midpoint);
            }
        }
        result
    }
}

static mut ACTIVE_INTERSECTIONS: IntersectionCache = IntersectionCache::EMPTY;

/// Gives the cooperative application loop exclusive access to pair geometry.
///
/// SAFETY: this private static is never touched by interrupt code, callbacks do
/// not re-enter this function, and references cannot escape the callback. The
/// renderer receives only the callback-bounded borrow after every requested
/// pair rebuild has completed.
pub fn with_intersections<R>(callback: impl FnOnce(&mut IntersectionCache) -> R) -> R {
    #[cfg(test)]
    let _guard = TEST_INTERSECTION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    unsafe { callback(&mut *core::ptr::addr_of_mut!(ACTIVE_INTERSECTIONS)) }
}

#[cfg(test)]
static TEST_INTERSECTION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scan_pair(
    surfaces: &SurfaceBank,
    first_shades: &crate::surface::TriangleShades,
    second_shades: &crate::surface::TriangleShades,
    first: usize,
    second: usize,
    mut emit: impl FnMut(PackedSegment, usize),
) -> usize {
    let mut total = 0usize;
    let mut row = 0;
    while row + 1 < surfaces.rows() {
        let mut column = 0;
        while column + 1 < surfaces.columns() {
            let mut triangle = 0;
            while triangle < 2 {
                if first_shades[row][column][triangle] != 0
                    && second_shades[row][column][triangle] != 0
                {
                    if let Some(segment) =
                        triangle_intersection(surfaces, first, second, row, column, triangle)
                    {
                        emit(segment, total);
                        total = total.saturating_add(1);
                    }
                }
                triangle += 1;
            }
            column += 1;
        }
        row += 1;
    }
    total
}

fn triangle_vertices(row: usize, column: usize, triangle: usize) -> [(usize, usize); 3] {
    if triangle == 0 {
        [(row, column), (row, column + 1), (row + 1, column + 1)]
    } else {
        [(row, column), (row + 1, column + 1), (row + 1, column)]
    }
}

fn triangle_intersection(
    surfaces: &SurfaceBank,
    first: usize,
    second: usize,
    row: usize,
    column: usize,
    triangle: usize,
) -> Option<PackedSegment> {
    let vertices = triangle_vertices(row, column, triangle);
    let mut differences = [0.0; 3];
    let mut index = 0;
    while index < 3 {
        differences[index] = surfaces
            .surface(first)?
            .height(vertices[index].0, vertices[index].1)
            - surfaces
                .surface(second)?
                .height(vertices[index].0, vertices[index].1);
        if !differences[index].is_finite() {
            return None;
        }
        index += 1;
    }
    let cell = row * (surfaces.columns() - 1) + column;
    let mut crossings = [PackedEndpoint(0); 3];
    let mut crossing_count = 0;
    let edges = [(0usize, 1usize), (1, 2), (2, 0)];
    let mut edge = 0;
    while edge < 3 {
        let (a, b) = edges[edge];
        let da = differences[a];
        let db = differences[b];
        if da == 0.0 && db == 0.0 {
            return Some(PackedSegment {
                first: pack_endpoint(cell, triangle, edge, 0),
                second: pack_endpoint(cell, triangle, edge, u16::MAX),
            });
        }
        let crossing = if da == 0.0 {
            Some(0)
        } else if db == 0.0 {
            Some(u16::MAX)
        } else if (da < 0.0) != (db < 0.0) {
            let denominator = da - db;
            let t = da / denominator;
            if t.is_finite() {
                Some((t.clamp(0.0, 1.0) * u16::MAX as f32 + 0.5) as u16)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(t) = crossing {
            let endpoint = pack_endpoint(cell, triangle, edge, t);
            let key = endpoint_grid_key(endpoint, surfaces.columns());
            let mut duplicate = false;
            let mut prior = 0;
            while prior < crossing_count {
                if endpoint_grid_key(crossings[prior], surfaces.columns()) == key {
                    duplicate = true;
                }
                prior += 1;
            }
            if !duplicate && crossing_count < crossings.len() {
                crossings[crossing_count] = endpoint;
                crossing_count += 1;
            }
        }
        edge += 1;
    }
    if crossing_count >= 2 {
        Some(PackedSegment {
            first: crossings[0],
            second: crossings[1],
        })
    } else if crossing_count == 1 {
        Some(PackedSegment {
            first: crossings[0],
            second: crossings[0],
        })
    } else {
        None
    }
}

fn pack_endpoint(cell: usize, triangle: usize, edge: usize, t: u16) -> PackedEndpoint {
    PackedEndpoint(
        t as u32
            | ((cell as u32 & 0x7ff) << 16)
            | ((triangle as u32 & 1) << 27)
            | ((edge as u32 & 3) << 28),
    )
}

fn unpack_endpoint(endpoint: PackedEndpoint) -> (usize, usize, usize, u16) {
    (
        ((endpoint.0 >> 16) & 0x7ff) as usize,
        ((endpoint.0 >> 27) & 1) as usize,
        ((endpoint.0 >> 28) & 3) as usize,
        endpoint.0 as u16,
    )
}

fn endpoint_grid_key(endpoint: PackedEndpoint, columns: usize) -> (u32, u32) {
    let (cell, triangle, edge, t) = unpack_endpoint(endpoint);
    let row = cell / (columns - 1);
    let column = cell % (columns - 1);
    let vertices = triangle_vertices(row, column, triangle);
    let edge_vertices = [(0usize, 1usize), (1, 2), (2, 0)];
    let (a, b) = edge_vertices[edge.min(2)];
    let tx = vertices[b].1 as i32 - vertices[a].1 as i32;
    let ty = vertices[b].0 as i32 - vertices[a].0 as i32;
    let x = vertices[a].1 as i32 * u16::MAX as i32 + tx * t as i32;
    let y = vertices[a].0 as i32 * u16::MAX as i32 + ty * t as i32;
    (x as u32, y as u32)
}

fn segment_grid_key(segment: PackedSegment, columns: usize) -> ((u32, u32), (u32, u32)) {
    let first = endpoint_grid_key(segment.first, columns);
    let second = endpoint_grid_key(segment.second, columns);
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn push_unique(output: &mut PairIntersections, candidate: PackedSegment, columns: usize) {
    if output.stored as usize >= MAX_INTERSECTION_SEGMENTS_PER_PAIR {
        return;
    }
    let candidate_key = segment_grid_key(candidate, columns);
    let mut index = 0;
    while index < output.stored as usize {
        if segment_grid_key(output.segments[index], columns) == candidate_key {
            return;
        }
        index += 1;
    }
    output.segments[output.stored as usize] = candidate;
    output.stored += 1;
}

pub fn reconstruct_endpoint(
    endpoint: PackedEndpoint,
    surfaces: &SurfaceBank,
    first: usize,
    second: usize,
) -> Option<Point3> {
    let (cell, triangle, edge, t) = unpack_endpoint(endpoint);
    let columns = surfaces.columns();
    if columns < 2 || cell >= (columns - 1) * (surfaces.rows() - 1) {
        return None;
    }
    let row = cell / (columns - 1);
    let column = cell % (columns - 1);
    let vertices = triangle_vertices(row, column, triangle);
    let edges = [(0usize, 1usize), (1, 2), (2, 0)];
    let (a, b) = edges[edge.min(2)];
    let va = vertices[a];
    let vb = vertices[b];
    let fraction = t as f32 / u16::MAX as f32;
    let x = surfaces.x(va.1) + (surfaces.x(vb.1) - surfaces.x(va.1)) * fraction;
    let y = surfaces.y(va.0) + (surfaces.y(vb.0) - surfaces.y(va.0)) * fraction;
    let first_surface = surfaces.surface(first)?;
    let second_surface = surfaces.surface(second)?;
    let zfa = first_surface.height(va.0, va.1);
    let zfb = first_surface.height(vb.0, vb.1);
    let zga = second_surface.height(va.0, va.1);
    let zgb = second_surface.height(vb.0, vb.1);
    let zf = zfa + (zfb - zfa) * fraction;
    let zg = zga + (zgb - zga) * fraction;
    let z = zf * 0.5 + zg * 0.5;
    if x.is_finite() && y.is_finite() && z.is_finite() {
        Some(Point3 { x, y, z })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::CompiledExpression;
    use crate::surface::{Domain, ResolutionPreset};

    #[test]
    fn fixed_capacities_and_pair_storage_are_bounded() {
        assert_eq!(MAX_TOTAL_INTERSECTION_SEGMENTS, 1536);
        assert_eq!(core::mem::size_of::<PackedSegment>(), 8);
        assert!(core::mem::size_of::<IntersectionCache>() <= 12_400);
    }

    #[test]
    fn planes_x_and_negative_x_intersect() {
        let first = CompiledExpression::compile("x").unwrap();
        let second = CompiledExpression::compile("-x").unwrap();
        let mut bank = SurfaceBank::EMPTY;
        bank.prepare_coordinates(Domain::DEFAULT, ResolutionPreset::Low);
        bank.resample_surface(0, &first);
        bank.resample_surface(1, &second);
        let mut cache = IntersectionCache::EMPTY;
        cache.rebuild_pair(0, &bank);
        assert!(cache.pair(0).unwrap().total() > 0);
        let representative = cache.representative(0, &bank).unwrap();
        assert!(representative.x.abs() < 0.01);
    }

    fn sampled_pair(first: &str, second: &str) -> (SurfaceBank, IntersectionCache) {
        let first = CompiledExpression::compile(first).unwrap();
        let second = CompiledExpression::compile(second).unwrap();
        let mut bank = SurfaceBank::EMPTY;
        bank.prepare_coordinates(Domain::DEFAULT, ResolutionPreset::Low);
        bank.resample_surface(0, &first);
        bank.resample_surface(1, &second);
        let mut cache = IntersectionCache::EMPTY;
        cache.initialize();
        cache.rebuild_pair(0, &bank);
        (bank, cache)
    }

    #[test]
    fn absent_tangent_invalid_and_overflow_cases_are_bounded() {
        let (_, absent) = sampled_pair("x", "x+1");
        assert_eq!(absent.pair(0).unwrap().total(), 0);

        let (_, tangent) = sampled_pair("x^2+y^2", "0");
        assert!(tangent
            .pair(0)
            .unwrap()
            .segments()
            .iter()
            .any(|segment| segment.first == segment.second));

        let (_, invalid) = sampled_pair("1/x", "-1/x");
        for segment in invalid.pair(0).unwrap().segments() {
            let first = endpoint_grid_key(segment.first, ResolutionPreset::Low.columns());
            let second = endpoint_grid_key(segment.second, ResolutionPreset::Low.columns());
            assert_ne!(first.0, 8 * u16::MAX as u32);
            assert_ne!(second.0, 8 * u16::MAX as u32);
        }

        let (_, coincident) = sampled_pair("x+y", "x+y");
        let data = coincident.pair(0).unwrap();
        assert!(data.truncated());
        assert!(data.segments().len() <= MAX_INTERSECTION_SEGMENTS_PER_PAIR);
    }

    #[test]
    fn rebuild_order_and_visibility_are_deterministic() {
        let (bank, mut first) = sampled_pair("x", "-x");
        let mut second = IntersectionCache::EMPTY;
        second.initialize();
        second.rebuild_pair(0, &bank);
        assert_eq!(
            first.pair(0).unwrap().segments(),
            second.pair(0).unwrap().segments()
        );
        let visibility = first.visibility_mask();
        first.toggle_visibility(0);
        assert_eq!(first.visibility_mask(), visibility ^ 1);
        assert!(first.pair(0).is_some());
        first.invalidate(1);
        assert!(first.pair(0).is_none());
    }

    #[test]
    fn curved_pair_produces_finite_centered_geometry() {
        let (bank, cache) = sampled_pair("x^2+y^2", "4");
        let data = cache.pair(0).unwrap();
        assert!(data.total() > 0);
        for segment in data.segments() {
            let first = reconstruct_endpoint(segment.first, &bank, 0, 1).unwrap();
            let second = reconstruct_endpoint(segment.second, &bank, 0, 1).unwrap();
            assert!(first.x.is_finite() && first.y.is_finite() && first.z.is_finite());
            assert!(second.x.is_finite() && second.y.is_finite() && second.z.is_finite());
            assert!((first.z - 4.0).abs() < 0.05);
            assert!((second.z - 4.0).abs() < 0.05);
        }
    }
}
