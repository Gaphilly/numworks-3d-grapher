//! Allocation-free wireframe and solid graph renderer for the 320×240 RGB565 display.
//!
//! The 24-pixel header leaves a 320×216 graph viewport. Rendering walks that
//! viewport in 320×8 bands (2,560 pixels / 5,120 bytes), producing exactly 27
//! display transfers without a 153,600-byte full-screen framebuffer or depth
//! buffer. Solid modes add only a same-sized 16-bit depth band; they never
//! allocate a full-screen color or depth buffer. The regular 25×19 height field
//! is traversed directly as 864 triangles rather than expanded into a mesh.
//!
//! Composition order is deliberately repeated inside every band: clear, grid,
//! axes, label backgrounds, numeric glyphs, surface, optional visible surface
//! grid, ticks/origin, axis glyphs, then push. Graph labels must remain in this
//! buffer. Drawing text directly to the display between band transfers can expose
//! stale positions or overwrite labels with later bands, reintroducing the
//! hardware flashing bug.

use crate::camera::{Camera, ProjectedLine, ProjectedPoint, Projector, ScreenPoint};
use crate::eadk::{self, Color, Point, Rect};
use crate::graph::{self, AxisVisibility, GraphOptions, RenderingMode, TickGenerator, PALETTE};
#[cfg(test)]
use crate::surface::TRIANGLES_PER_CELL;
use crate::surface::{Domain, Point3, SurfaceGrid, TriangleShades, COLUMNS, ROWS};

const SCREEN_WIDTH: usize = 320;
const SCREEN_HEIGHT: usize = 240;
const BAND_HEIGHT: usize = 8;
const GRAPH_TOP: usize = 24;
const MAX_WORLD_LINES: usize = 48;
const MAX_LABELS: usize = 12;
const LABEL_NEAR_DEPTH: f32 = 1.05;
const GLYPH_WIDTH: i32 = 5;
const GLYPH_HEIGHT: i32 = 7;
const GLYPH_SPACING: i32 = 1;
const LABEL_PADDING: i32 = 1;
const LABEL_SEPARATION: i32 = 2;
const MAX_NUMERIC_LABELS_PER_AXIS: usize = 3;
const MIN_NUMERIC_SURFACE_DISTANCE_SQUARED: i32 = 9;
const MIN_AXIS_SURFACE_DISTANCE_SQUARED: i32 = 4;
const SOLID_NEAR_DEPTH: f32 = 1.05;
const DEPTH_KEY_MAX: f32 = u16::MAX as f32;
#[cfg(test)]
const SURFACE_GRID_DEPTH_TOLERANCE: u16 = 24;
#[cfg(test)]
const SURFACE_GRID_EDGE_COUNT: usize = ROWS * (COLUMNS - 1) + COLUMNS * (ROWS - 1);

const NUMERIC_GLYPHS: [[u8; 7]; 13] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
    ],
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    [
        0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
    ],
    [
        0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
    ],
    [
        0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000,
    ],
    [
        0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00110, 0b00110,
    ],
    [
        0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
    ],
];
const AXIS_X_GLYPH: [u8; 7] = [
    0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
];
const AXIS_Y_GLYPH: [u8; 7] = [
    0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
];
const AXIS_Z_GLYPH: [u8; 7] = [
    0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
];

// Solid shading is computed once per triangle, but the same shade is consumed
// once for every band that triangle can touch. Keeping the exact RGB565 result
// in flash avoids repeating three channel multiplications/divisions in the
// 27-band loop without adding RAM or changing the lighting appearance.
const SHADE_COLOR_LUT: [u16; 256] = build_shade_color_lut();

const fn build_shade_color_lut() -> [u16; 256] {
    let red = ((PALETTE.solid_surface.rgb565 >> 11) & 0x1f) as u32;
    let green = ((PALETTE.solid_surface.rgb565 >> 5) & 0x3f) as u32;
    let blue = (PALETTE.solid_surface.rgb565 & 0x1f) as u32;
    let mut colors = [0_u16; 256];
    let mut shade = 0_u32;
    while shade < colors.len() as u32 {
        colors[shade as usize] = ((red * shade / 255) as u16) << 11
            | ((green * shade / 255) as u16) << 5
            | (blue * shade / 255) as u16;
        shade += 1;
    }
    colors
}

#[derive(Clone, Copy, PartialEq)]
enum LineLayer {
    Grid,
    Axis,
    Tick,
}

#[derive(Clone, Copy)]
struct ColoredLine {
    line: ProjectedLine,
    color: Color,
    layer: LineLayer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LabelKind {
    AxisX,
    AxisY,
    AxisZ,
    Number(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Label {
    anchor: ScreenPoint,
    kind: LabelKind,
    color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct LabelRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Clone, Copy)]
struct NumericCandidate {
    projected: ProjectedPoint,
    value: f32,
    surface_distance_squared: i32,
}

struct NumericCandidates {
    entries: [Option<NumericCandidate>; graph::MAX_TICKS],
    count: usize,
}

impl NumericCandidates {
    fn new() -> NumericCandidates {
        NumericCandidates {
            entries: [None; graph::MAX_TICKS],
            count: 0,
        }
    }

    fn push(&mut self, candidate: NumericCandidate) {
        if self.count < self.entries.len() {
            self.entries[self.count] = Some(candidate);
            self.count += 1;
        }
    }
}

struct WorldGeometry {
    // All auxiliary geometry/labels has fixed capacity. Overflow is conservatively
    // dropped rather than allocating or risking an out-of-bounds firmware call.
    lines: [Option<ColoredLine>; MAX_WORLD_LINES],
    line_count: usize,
    labels: [Option<Label>; MAX_LABELS],
    label_count: usize,
    origin: Option<ScreenPoint>,
}

/// Compact projected sample used only by solid modes. A zero reciprocal-depth
/// key is invalid; otherwise larger keys are nearer to the camera. Keeping the
/// key quantized avoids another 1,900-byte `f32` array.
#[derive(Clone, Copy)]
#[repr(C)]
struct SolidVertex {
    screen: ScreenPoint,
    inverse_depth: u16,
}

impl SolidVertex {
    const INVALID: SolidVertex = SolidVertex {
        screen: ScreenPoint::INVALID,
        inverse_depth: 0,
    };

    fn is_visible(self) -> bool {
        self.screen.is_visible() && self.inverse_depth != 0
    }
}

#[derive(Clone, Copy)]
enum ProjectedSurface<'a> {
    Wireframe(&'a [[ScreenPoint; COLUMNS]; ROWS]),
    Solid(&'a [[SolidVertex; COLUMNS]; ROWS]),
}

impl ProjectedSurface<'_> {
    fn screen(self, row: usize, column: usize) -> ScreenPoint {
        if row >= ROWS || column >= COLUMNS {
            return ScreenPoint::INVALID;
        }
        match self {
            ProjectedSurface::Wireframe(points) => points[row][column],
            ProjectedSurface::Solid(points) => points[row][column].screen,
        }
    }
}

impl WorldGeometry {
    fn new() -> WorldGeometry {
        WorldGeometry {
            lines: [None; MAX_WORLD_LINES],
            line_count: 0,
            labels: [None; MAX_LABELS],
            label_count: 0,
            origin: None,
        }
    }

    fn add_line(&mut self, line: Option<ProjectedLine>, color: Color, layer: LineLayer) {
        if let Some(line) = line {
            let candidate = ColoredLine { line, color, layer };
            if self.line_count < MAX_WORLD_LINES {
                self.lines[self.line_count] = Some(candidate);
                self.line_count += 1;
                return;
            }

            // The auxiliary grid is intentionally expendable. If a bounded
            // tick/grid configuration ever fills this array, keep X/Y/Z axes
            // visible by replacing one grid line rather than silently dropping
            // the mathematical coordinate system.
            if layer == LineLayer::Axis {
                let mut index = 0;
                while index < self.line_count {
                    if matches!(
                        self.lines[index],
                        Some(ColoredLine {
                            layer: LineLayer::Grid,
                            ..
                        })
                    ) {
                        self.lines[index] = Some(candidate);
                        return;
                    }
                    index += 1;
                }
            }
        }
    }

    fn add_label(
        &mut self,
        projected: Option<ProjectedPoint>,
        kind: LabelKind,
        color: Color,
    ) -> bool {
        let projected = match projected {
            Some(projected) => projected,
            None => return false,
        };
        let candidate_rect = match label_rect(projected.screen, kind) {
            Some(rect) => rect,
            None => return false,
        };
        if self.label_count >= MAX_LABELS || projected.depth < LABEL_NEAR_DEPTH {
            return false;
        }

        let candidate = Label {
            anchor: projected.screen,
            kind,
            color,
        };
        let mut index = 0;
        while index < self.label_count {
            if let Some(existing) = self.labels[index] {
                if existing == candidate {
                    return false;
                }
                if let Some(existing_rect) = label_rect(existing.anchor, existing.kind) {
                    if label_rects_overlap(candidate_rect, existing_rect, LABEL_SEPARATION) {
                        return false;
                    }
                }
            }
            index += 1;
        }
        self.labels[self.label_count] = Some(candidate);
        self.label_count += 1;
        true
    }
}

/// Projects cached world geometry and renders a complete graph viewport.
///
/// Camera-only redraws reuse `surface`; expression evaluation belongs to the
/// surface-dirty path in `main`, not this projection/rasterization function.
/// Wireframe and solid paths are separate so wireframe never pays the solid
/// mode's 5,120-byte depth-band stack cost.
pub fn render(
    camera: &Camera,
    domain: Domain,
    surface: &SurfaceGrid,
    options: GraphOptions,
    diagnostics_enabled: bool,
) {
    match options.rendering_mode {
        RenderingMode::Wireframe => render_wireframe(camera, domain, surface, options),
        RenderingMode::Solid => render_solid(camera, domain, surface, options, diagnostics_enabled),
    }
}

// Prevent release LTO from merging the mutually exclusive stack frames: the
// wireframe call must never reserve the depth/lighting storage used by solid.
#[inline(never)]
fn render_wireframe(camera: &Camera, domain: Domain, surface: &SurfaceGrid, options: GraphOptions) {
    // A sentinel-based cache is smaller than an Option-rich structure and lets
    // both row and column wire passes reuse every projected sample.
    let mut projected = [[ScreenPoint::INVALID; COLUMNS]; ROWS];
    let projector = camera.projector();
    let (z_min, z_max, has_height) = surface.z_range();
    let mut row = 0;
    while row < ROWS {
        let mut column = 0;
        while column < COLUMNS {
            let point = surface.point(domain, column, row);
            projected[row][column] = projector.project(point);
            column += 1;
        }
        row += 1;
    }

    let geometry = build_world_geometry(
        &projector,
        domain,
        z_min,
        z_max,
        has_height,
        ProjectedSurface::Wireframe(&projected),
        options,
    );

    eadk::display::wait_for_vblank();
    let mut pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
    let mut band_y = GRAPH_TOP;
    while band_y < SCREEN_HEIGHT {
        // Do not move any direct-display operation into this sequence. Each band
        // must be a complete slice of one camera state before its single push.
        pixels.fill(PALETTE.background);
        draw_geometry_lines(&mut pixels, band_y, &geometry, LineLayer::Grid);
        draw_geometry_lines(&mut pixels, band_y, &geometry, LineLayer::Axis);
        draw_label_backgrounds(&mut pixels, band_y, &geometry);
        draw_labels(&mut pixels, band_y, &geometry, true);
        draw_wireframe_surface(&mut pixels, band_y, &projected);
        draw_geometry_lines(&mut pixels, band_y, &geometry, LineLayer::Tick);
        draw_origin(&mut pixels, band_y, geometry.origin);
        draw_labels(&mut pixels, band_y, &geometry, false);

        eadk::display::push_rect(
            Rect {
                x: 0,
                y: band_y as u16,
                width: SCREEN_WIDTH as u16,
                height: BAND_HEIGHT as u16,
            },
            &pixels,
        );
        band_y += BAND_HEIGHT;
    }
}

#[inline(never)]
fn render_solid(
    camera: &Camera,
    domain: Domain,
    surface: &SurfaceGrid,
    options: GraphOptions,
    diagnostics_enabled: bool,
) {
    diagnostic_marker(diagnostics_enabled, b"P00\0");
    // Lighting is cached at surface-sampling time. This Solid-only path avoids
    // recomputing 864 normals/square roots/divisions on camera-only redraws.
    let triangle_shades = surface.triangle_shades();
    let mut projected = [[SolidVertex::INVALID; COLUMNS]; ROWS];
    let projector = camera.projector();
    let (z_min, z_max, has_height) = surface.z_range();
    let mut row = 0;
    while row < ROWS {
        let mut column = 0;
        while column < COLUMNS {
            let point = surface.solid_point(column, row);
            if let Some(point) = projector.project_with_depth(point) {
                let inverse_depth = encode_inverse_depth(point.depth);
                if inverse_depth != 0 {
                    projected[row][column] = SolidVertex {
                        screen: point.screen,
                        inverse_depth,
                    };
                }
            }
            column += 1;
        }
        row += 1;
    }

    let geometry = build_world_geometry(
        &projector,
        domain,
        z_min,
        z_max,
        has_height,
        ProjectedSurface::Solid(&projected),
        options,
    );

    eadk::display::wait_for_vblank();
    let mut pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
    let mut depth = [0_u16; SCREEN_WIDTH * BAND_HEIGHT];
    let mut band_y = GRAPH_TOP;
    while band_y < SCREEN_HEIGHT {
        // The color and depth slices describe the same eight physical rows and
        // are both reset before composing that band from one camera state.
        pixels.fill(PALETTE.background);
        depth.fill(0);
        draw_geometry_lines(&mut pixels, band_y, &geometry, LineLayer::Grid);
        draw_geometry_lines(&mut pixels, band_y, &geometry, LineLayer::Axis);
        draw_label_backgrounds(&mut pixels, band_y, &geometry);
        draw_labels(&mut pixels, band_y, &geometry, true);
        diagnostic_marker_band(diagnostics_enabled, b'F', band_y);
        draw_solid_surface(&mut pixels, &mut depth, band_y, &projected, triangle_shades);
        draw_geometry_lines(&mut pixels, band_y, &geometry, LineLayer::Tick);
        draw_origin(&mut pixels, band_y, geometry.origin);
        draw_labels(&mut pixels, band_y, &geometry, false);

        diagnostic_marker_band(diagnostics_enabled, b'D', band_y);
        eadk::display::push_rect(
            Rect {
                x: 0,
                y: band_y as u16,
                width: SCREEN_WIDTH as u16,
                height: BAND_HEIGHT as u16,
            },
            &pixels,
        );
        band_y += BAND_HEIGHT;
    }
    diagnostic_marker(diagnostics_enabled, b"OK\0");
}

// EADK offers neither a serial console nor persistent crash logs. The tiny
// marker survives a renderer stall long enough to identify the active phase:
// P=setup/projection, F=solid fill, D=band transfer.
// Its two digits are the current band (00..26). It only touches Graph's spare
// header space while diagnostic mode is enabled.
fn diagnostic_marker(enabled: bool, text: &[u8]) {
    if enabled {
        eadk::display::draw_string(
            text,
            Point { x: 286, y: 4 },
            false,
            Color { rgb565: 0xffff },
            Color { rgb565: 0x245f },
        );
    }
}

fn diagnostic_marker_band(enabled: bool, phase: u8, band_y: usize) {
    if !enabled {
        return;
    }
    let band = (band_y - GRAPH_TOP) / BAND_HEIGHT;
    let text = [phase, b'0' + (band / 10) as u8, b'0' + (band % 10) as u8, 0];
    diagnostic_marker(true, &text);
}

fn build_world_geometry(
    projector: &Projector,
    domain: Domain,
    surface_z_min: f32,
    surface_z_max: f32,
    has_height: bool,
    projected_surface: ProjectedSurface<'_>,
    options: GraphOptions,
) -> WorldGeometry {
    let mut geometry = WorldGeometry::new();
    let visibility = graph::axes_for_domain(domain);
    let (z_min, z_max) = z_axis_range(surface_z_min, surface_z_max, has_height);
    if options.show_grid {
        add_grid(&mut geometry, projector, domain);
    }
    if options.show_axes {
        add_axes(
            &mut geometry,
            projector,
            domain,
            z_min,
            z_max,
            visibility,
            projected_surface,
            options,
        );
    }
    if options.show_axes && visibility.z {
        geometry.origin = visible_point(projector.project(Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }));
    }
    geometry
}

fn add_grid(geometry: &mut WorldGeometry, projector: &Projector, domain: Domain) {
    let mut x_ticks = TickGenerator::new(domain.x_min, domain.x_max);
    while let Some(x) = x_ticks.next() {
        if x != 0.0 {
            geometry.add_line(
                projector.project_line(
                    Point3 {
                        x,
                        y: domain.y_min,
                        z: 0.0,
                    },
                    Point3 {
                        x,
                        y: domain.y_max,
                        z: 0.0,
                    },
                ),
                PALETTE.grid,
                LineLayer::Grid,
            );
        }
    }
    let mut y_ticks = TickGenerator::new(domain.y_min, domain.y_max);
    while let Some(y) = y_ticks.next() {
        if y != 0.0 {
            geometry.add_line(
                projector.project_line(
                    Point3 {
                        x: domain.x_min,
                        y,
                        z: 0.0,
                    },
                    Point3 {
                        x: domain.x_max,
                        y,
                        z: 0.0,
                    },
                ),
                PALETTE.grid,
                LineLayer::Grid,
            );
        }
    }
}

fn add_axes(
    geometry: &mut WorldGeometry,
    projector: &Projector,
    domain: Domain,
    z_min: f32,
    z_max: f32,
    visibility: AxisVisibility,
    projected_surface: ProjectedSurface<'_>,
    options: GraphOptions,
) {
    let tick_size = domain_tick_size(domain);
    if visibility.x {
        geometry.add_line(
            projector.project_line(
                Point3 {
                    x: domain.x_min,
                    y: 0.0,
                    z: 0.0,
                },
                Point3 {
                    x: domain.x_max,
                    y: 0.0,
                    z: 0.0,
                },
            ),
            PALETTE.x_axis,
            LineLayer::Axis,
        );
        if options.show_ticks || options.show_labels {
            add_x_ticks(
                geometry,
                projector,
                domain,
                tick_size,
                projected_surface,
                options,
            );
        }
        if options.show_labels {
            let projected = projector.project_with_depth(Point3 {
                x: domain.x_max,
                y: 0.0,
                z: 0.0,
            });
            add_axis_label(
                geometry,
                projected,
                LabelKind::AxisX,
                PALETTE.x_axis,
                projected_surface,
            );
        }
    }
    if visibility.y {
        geometry.add_line(
            projector.project_line(
                Point3 {
                    x: 0.0,
                    y: domain.y_min,
                    z: 0.0,
                },
                Point3 {
                    x: 0.0,
                    y: domain.y_max,
                    z: 0.0,
                },
            ),
            PALETTE.y_axis,
            LineLayer::Axis,
        );
        if options.show_ticks || options.show_labels {
            add_y_ticks(
                geometry,
                projector,
                domain,
                tick_size,
                projected_surface,
                options,
            );
        }
        if options.show_labels {
            let projected = projector.project_with_depth(Point3 {
                x: 0.0,
                y: domain.y_max,
                z: 0.0,
            });
            add_axis_label(
                geometry,
                projected,
                LabelKind::AxisY,
                PALETTE.y_axis,
                projected_surface,
            );
        }
    }
    if visibility.z {
        geometry.add_line(
            projector.project_line(
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: z_min,
                },
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: z_max,
                },
            ),
            PALETTE.z_axis,
            LineLayer::Axis,
        );
        if options.show_ticks || options.show_labels {
            add_z_ticks(
                geometry,
                projector,
                z_min,
                z_max,
                tick_size,
                projected_surface,
                options,
            );
        }
        if options.show_labels {
            let projected = projector.project_with_depth(Point3 {
                x: 0.0,
                y: 0.0,
                z: z_max,
            });
            add_axis_label(
                geometry,
                projected,
                LabelKind::AxisZ,
                PALETTE.z_axis,
                projected_surface,
            );
        }
    }
}

fn add_x_ticks(
    geometry: &mut WorldGeometry,
    projector: &Projector,
    domain: Domain,
    tick_size: f32,
    projected_surface: ProjectedSurface<'_>,
    options: GraphOptions,
) {
    let mut ticks = TickGenerator::new(domain.x_min, domain.x_max);
    let mut candidates = NumericCandidates::new();
    while let Some(x) = ticks.next() {
        if options.show_labels {
            consider_numeric_candidate(
                &mut candidates,
                projector.project_with_depth(Point3 {
                    x,
                    y: -tick_size * 1.6,
                    z: 0.0,
                }),
                x,
                projected_surface,
            );
        }
        if options.show_ticks && x != 0.0 {
            geometry.add_line(
                projector.project_line(
                    Point3 {
                        x,
                        y: -tick_size,
                        z: 0.0,
                    },
                    Point3 {
                        x,
                        y: tick_size,
                        z: 0.0,
                    },
                ),
                PALETTE.x_axis,
                LineLayer::Tick,
            );
        }
    }
    if options.show_labels {
        select_numeric_labels(geometry, &mut candidates);
    }
}

fn add_y_ticks(
    geometry: &mut WorldGeometry,
    projector: &Projector,
    domain: Domain,
    tick_size: f32,
    projected_surface: ProjectedSurface<'_>,
    options: GraphOptions,
) {
    let mut ticks = TickGenerator::new(domain.y_min, domain.y_max);
    let mut candidates = NumericCandidates::new();
    while let Some(y) = ticks.next() {
        if options.show_labels {
            consider_numeric_candidate(
                &mut candidates,
                projector.project_with_depth(Point3 {
                    x: tick_size * 1.6,
                    y,
                    z: 0.0,
                }),
                y,
                projected_surface,
            );
        }
        if options.show_ticks && y != 0.0 {
            geometry.add_line(
                projector.project_line(
                    Point3 {
                        x: -tick_size,
                        y,
                        z: 0.0,
                    },
                    Point3 {
                        x: tick_size,
                        y,
                        z: 0.0,
                    },
                ),
                PALETTE.y_axis,
                LineLayer::Tick,
            );
        }
    }
    if options.show_labels {
        select_numeric_labels(geometry, &mut candidates);
    }
}

fn add_z_ticks(
    geometry: &mut WorldGeometry,
    projector: &Projector,
    z_min: f32,
    z_max: f32,
    tick_size: f32,
    projected_surface: ProjectedSurface<'_>,
    options: GraphOptions,
) {
    let mut ticks = TickGenerator::new(z_min, z_max);
    let mut candidates = NumericCandidates::new();
    while let Some(z) = ticks.next() {
        if options.show_labels {
            consider_numeric_candidate(
                &mut candidates,
                projector.project_with_depth(Point3 {
                    x: tick_size * 1.6,
                    y: 0.0,
                    z,
                }),
                z,
                projected_surface,
            );
        }
        if options.show_ticks && z != 0.0 {
            geometry.add_line(
                projector.project_line(
                    Point3 {
                        x: -tick_size,
                        y: 0.0,
                        z,
                    },
                    Point3 {
                        x: tick_size,
                        y: 0.0,
                        z,
                    },
                ),
                PALETTE.z_axis,
                LineLayer::Tick,
            );
        }
    }
    if options.show_labels {
        select_numeric_labels(geometry, &mut candidates);
    }
}

fn consider_numeric_candidate(
    candidates: &mut NumericCandidates,
    projected: Option<ProjectedPoint>,
    value: f32,
    projected_surface: ProjectedSurface<'_>,
) {
    let projected = match projected {
        Some(projected) => projected,
        None => return,
    };
    let rect = match label_rect(projected.screen, LabelKind::Number(value)) {
        Some(rect) => rect,
        None => return,
    };
    let surface_distance_squared = surface_distance_squared(rect, projected_surface);
    if projected.depth >= LABEL_NEAR_DEPTH
        && surface_distance_squared >= MIN_NUMERIC_SURFACE_DISTANCE_SQUARED
    {
        candidates.push(NumericCandidate {
            projected,
            value,
            surface_distance_squared,
        });
    }
}

fn select_numeric_labels(geometry: &mut WorldGeometry, candidates: &mut NumericCandidates) {
    let mut selected_points = [None; MAX_NUMERIC_LABELS_PER_AXIS];
    let mut selected_count = 0;
    while selected_count < MAX_NUMERIC_LABELS_PER_AXIS {
        let mut best_index = None;
        let mut best_score = -1_i32;
        let mut index = 0;
        while index < candidates.count {
            if let Some(candidate) = candidates.entries[index] {
                let separation = minimum_selected_distance_squared(
                    candidate.projected.screen,
                    &selected_points,
                    selected_count,
                );
                let score = candidate.surface_distance_squared.min(4096) + separation.min(4096);
                if score > best_score {
                    best_score = score;
                    best_index = Some(index);
                }
            }
            index += 1;
        }
        let index = match best_index {
            Some(index) => index,
            None => break,
        };
        let candidate = match candidates.entries[index].take() {
            Some(candidate) => candidate,
            None => break,
        };
        if geometry.add_label(
            Some(candidate.projected),
            LabelKind::Number(candidate.value),
            PALETTE.text,
        ) {
            selected_points[selected_count] = Some(candidate.projected.screen);
            selected_count += 1;
        }
    }
}

fn minimum_selected_distance_squared(
    point: ScreenPoint,
    selected: &[Option<ScreenPoint>; MAX_NUMERIC_LABELS_PER_AXIS],
    count: usize,
) -> i32 {
    if count == 0 {
        let dx = point.x as i32 - SCREEN_WIDTH as i32 / 2;
        let dy = point.y as i32 - (GRAPH_TOP as i32 + SCREEN_HEIGHT as i32) / 2;
        return dx * dx + dy * dy;
    }
    let mut minimum = i32::MAX;
    let mut index = 0;
    while index < count {
        if let Some(selected) = selected[index] {
            let dx = point.x as i32 - selected.x as i32;
            let dy = point.y as i32 - selected.y as i32;
            let distance = dx * dx + dy * dy;
            if distance < minimum {
                minimum = distance;
            }
        }
        index += 1;
    }
    minimum
}

fn add_axis_label(
    geometry: &mut WorldGeometry,
    projected: Option<ProjectedPoint>,
    kind: LabelKind,
    color: Color,
    projected_surface: ProjectedSurface<'_>,
) {
    let projected = match projected {
        Some(projected) => projected,
        None => return,
    };
    let rect = match label_rect(projected.screen, kind) {
        Some(rect) => rect,
        None => return,
    };
    if surface_distance_squared(rect, projected_surface) >= MIN_AXIS_SURFACE_DISTANCE_SQUARED {
        let _ = geometry.add_label(Some(projected), kind, color);
    }
}

fn draw_wireframe_surface(
    pixels: &mut [Color],
    band_y: usize,
    projected: &[[ScreenPoint; COLUMNS]; ROWS],
) {
    let mut row = 0;
    while row < ROWS {
        let mut column = 0;
        while column < COLUMNS {
            if column + 1 < COLUMNS {
                draw_line(
                    pixels,
                    band_y,
                    projected[row][column],
                    projected[row][column + 1],
                    PALETTE.surface,
                );
            }
            if row + 1 < ROWS {
                draw_line(
                    pixels,
                    band_y,
                    projected[row][column],
                    projected[row + 1][column],
                    PALETTE.surface,
                );
            }
            column += 1;
        }
        row += 1;
    }
}

fn draw_solid_surface(
    pixels: &mut [Color],
    depth: &mut [u16],
    band_y: usize,
    projected: &[[SolidVertex; COLUMNS]; ROWS],
    triangle_shades: &TriangleShades,
) {
    let mut row = 0;
    while row + 1 < ROWS {
        let mut column = 0;
        while column + 1 < COLUMNS {
            let shade = triangle_shades[row][column][0];
            if shade != 0 {
                let first = projected[row][column];
                let second = projected[row][column + 1];
                let third = projected[row + 1][column + 1];
                if triangle_intersects_band(first, second, third, band_y) {
                    draw_triangle_band(
                        pixels,
                        depth,
                        band_y,
                        [first, second, third],
                        shaded_surface_color(shade),
                    );
                }
            }
            let shade = triangle_shades[row][column][1];
            if shade != 0 {
                let first = projected[row][column];
                let second = projected[row + 1][column + 1];
                let third = projected[row + 1][column];
                if triangle_intersects_band(first, second, third, band_y) {
                    draw_triangle_band(
                        pixels,
                        depth,
                        band_y,
                        [first, second, third],
                        shaded_surface_color(shade),
                    );
                }
            }
            column += 1;
        }
        row += 1;
    }
}

/// Rejects a triangle before color conversion or raster setup when its projected
/// Y extent cannot touch the active physical display band. It intentionally
/// does not decide visibility: the rasterizer retains that validation so direct
/// callers and invalid-projection handling keep their existing behavior.
#[inline]
fn triangle_intersects_band(
    first: SolidVertex,
    second: SolidVertex,
    third: SolidVertex,
    band_y: usize,
) -> bool {
    let minimum_y = minimum3(
        first.screen.y as i32,
        second.screen.y as i32,
        third.screen.y as i32,
    );
    let maximum_y = maximum3(
        first.screen.y as i32,
        second.screen.y as i32,
        third.screen.y as i32,
    );
    maximum_y >= band_y as i32 && minimum_y < (band_y + BAND_HEIGHT) as i32
}

/// Fills one projected triangle only inside the active eight-row band.
///
/// Integer edge equations use doubled pixel-center coordinates and are stepped
/// with additions across each row. The reciprocal-depth plane is likewise
/// initialized once and incremented in X/Y. This avoids three edge-function
/// evaluations and a division for every covered pixel on the calculator CPU.
fn draw_triangle_band(
    pixels: &mut [Color],
    depth: &mut [u16],
    band_y: usize,
    mut vertices: [SolidVertex; 3],
    color: Color,
) {
    if pixels.len() < SCREEN_WIDTH * BAND_HEIGHT
        || depth.len() < SCREEN_WIDTH * BAND_HEIGHT
        || !vertices[0].is_visible()
        || !vertices[1].is_visible()
        || !vertices[2].is_visible()
    {
        return;
    }

    if !triangle_intersects_band(vertices[0], vertices[1], vertices[2], band_y) {
        return;
    }

    let mut area = edge_at_vertex(vertices[0].screen, vertices[1].screen, vertices[2].screen);
    if area == 0 {
        return;
    }
    if area < 0 {
        vertices.swap(1, 2);
        area = -area;
    }

    let minimum_x = minimum3(
        vertices[0].screen.x as i32,
        vertices[1].screen.x as i32,
        vertices[2].screen.x as i32,
    )
    .max(0);
    let maximum_x = maximum3(
        vertices[0].screen.x as i32,
        vertices[1].screen.x as i32,
        vertices[2].screen.x as i32,
    )
    .min(SCREEN_WIDTH as i32 - 1);
    let minimum_y = minimum3(
        vertices[0].screen.y as i32,
        vertices[1].screen.y as i32,
        vertices[2].screen.y as i32,
    )
    .max(band_y as i32);
    let maximum_y = maximum3(
        vertices[0].screen.y as i32,
        vertices[1].screen.y as i32,
        vertices[2].screen.y as i32,
    )
    .min((band_y + BAND_HEIGHT - 1) as i32);
    if minimum_x > maximum_x || minimum_y > maximum_y {
        return;
    }

    // Each pixel-center edge value is twice its ordinary integer-coordinate
    // value, so the three barycentric weights sum to `2 * area`.
    let doubled_area = area.saturating_mul(2);
    if doubled_area <= 0 {
        return;
    }
    let reciprocal_area = 1.0 / doubled_area as f32;

    let edge_0_x = -2 * (vertices[2].screen.y as i32 - vertices[1].screen.y as i32);
    let edge_1_x = -2 * (vertices[0].screen.y as i32 - vertices[2].screen.y as i32);
    let edge_2_x = -2 * (vertices[1].screen.y as i32 - vertices[0].screen.y as i32);
    let edge_0_y = 2 * (vertices[2].screen.x as i32 - vertices[1].screen.x as i32);
    let edge_1_y = 2 * (vertices[0].screen.x as i32 - vertices[2].screen.x as i32);
    let edge_2_y = 2 * (vertices[1].screen.x as i32 - vertices[0].screen.x as i32);

    let mut row_weight_0 =
        edge_at_pixel_center(vertices[1].screen, vertices[2].screen, minimum_x, minimum_y);
    let mut row_weight_1 =
        edge_at_pixel_center(vertices[2].screen, vertices[0].screen, minimum_x, minimum_y);
    let mut row_weight_2 =
        edge_at_pixel_center(vertices[0].screen, vertices[1].screen, minimum_x, minimum_y);
    let depth_0 = vertices[0].inverse_depth as f32;
    let depth_1 = vertices[1].inverse_depth as f32;
    let depth_2 = vertices[2].inverse_depth as f32;
    let mut row_depth = (row_weight_0 as f32 * depth_0
        + row_weight_1 as f32 * depth_1
        + row_weight_2 as f32 * depth_2)
        * reciprocal_area;
    let depth_step_x =
        (edge_0_x as f32 * depth_0 + edge_1_x as f32 * depth_1 + edge_2_x as f32 * depth_2)
            * reciprocal_area;
    let depth_step_y =
        (edge_0_y as f32 * depth_0 + edge_1_y as f32 * depth_1 + edge_2_y as f32 * depth_2)
            * reciprocal_area;

    let mut y = minimum_y;
    while y <= maximum_y {
        let mut weight_0 = row_weight_0;
        let mut weight_1 = row_weight_1;
        let mut weight_2 = row_weight_2;
        let mut interpolated_depth = row_depth;
        let mut x = minimum_x;
        while x <= maximum_x {
            if weight_0 >= 0 && weight_1 >= 0 && weight_2 >= 0 {
                if interpolated_depth.is_finite() && interpolated_depth > 0.0 {
                    let key = interpolated_depth.min(DEPTH_KEY_MAX) as u16;
                    let local_y = y as usize - band_y;
                    let index = local_y * SCREEN_WIDTH + x as usize;
                    // Keep the guard at the FFI-facing rendering boundary. It
                    // costs little compared with a pixel write and prevents a
                    // malformed projection from panicking the calculator.
                    if index < depth.len() && key > depth[index] {
                        depth[index] = key;
                        pixels[index] = color;
                    }
                }
            }
            weight_0 += edge_0_x;
            weight_1 += edge_1_x;
            weight_2 += edge_2_x;
            interpolated_depth += depth_step_x;
            x += 1;
        }
        row_weight_0 += edge_0_y;
        row_weight_1 += edge_1_y;
        row_weight_2 += edge_2_y;
        row_depth += depth_step_y;
        y += 1;
    }
}

#[cfg(test)]
fn draw_depth_surface_grid(
    pixels: &mut [Color],
    depth: &[u16],
    band_y: usize,
    projected: &[[SolidVertex; COLUMNS]; ROWS],
    triangle_shades: &TriangleShades,
) {
    let mut row = 0;
    while row < ROWS {
        let mut column = 0;
        while column < COLUMNS {
            // The overlay is intentionally sparse: retain the outer boundary
            // and every second interior row/column to bound camera-motion work.
            let horizontal_overlay = row == 0 || row + 1 == ROWS || row % 2 == 0;
            let vertical_overlay = column == 0 || column + 1 == COLUMNS || column % 2 == 0;
            if horizontal_overlay
                && column + 1 < COLUMNS
                && horizontal_surface_edge_is_valid(triangle_shades, row, column)
            {
                draw_depth_line(
                    pixels,
                    depth,
                    band_y,
                    projected[row][column],
                    projected[row][column + 1],
                    PALETTE.grid,
                );
            }
            if vertical_overlay
                && row + 1 < ROWS
                && vertical_surface_edge_is_valid(triangle_shades, row, column)
            {
                draw_depth_line(
                    pixels,
                    depth,
                    band_y,
                    projected[row][column],
                    projected[row + 1][column],
                    PALETTE.grid,
                );
            }
            column += 1;
        }
        row += 1;
    }
}

#[cfg(test)]
fn horizontal_surface_edge_is_valid(
    triangle_shades: &TriangleShades,
    row: usize,
    column: usize,
) -> bool {
    if row >= ROWS || column + 1 >= COLUMNS {
        return false;
    }
    (row + 1 < ROWS && triangle_shades[row][column][0] != 0)
        || (row > 0 && triangle_shades[row - 1][column][1] != 0)
}

#[cfg(test)]
fn vertical_surface_edge_is_valid(
    triangle_shades: &TriangleShades,
    row: usize,
    column: usize,
) -> bool {
    if row + 1 >= ROWS || column >= COLUMNS {
        return false;
    }
    (column + 1 < COLUMNS && triangle_shades[row][column][1] != 0)
        || (column > 0 && triangle_shades[row][column - 1][0] != 0)
}

/// Draws only portions of a surface edge whose reciprocal depth matches the
/// visible fill. Back-facing grid edges therefore do not show through solid
/// triangles and turn the result back into an opaque wireframe.
#[cfg(test)]
fn draw_depth_line(
    pixels: &mut [Color],
    depth: &[u16],
    band_y: usize,
    start: SolidVertex,
    end: SolidVertex,
    color: Color,
) {
    if !start.is_visible() || !end.is_visible() {
        return;
    }
    let band_bottom = band_y as i32 + BAND_HEIGHT as i32 - 1;
    let min_y = (start.screen.y.min(end.screen.y)) as i32;
    let max_y = (start.screen.y.max(end.screen.y)) as i32;
    if max_y < band_y as i32 || min_y > band_bottom {
        return;
    }

    let x0 = start.screen.x as i32;
    let y0 = start.screen.y as i32;
    let x1 = end.screen.x as i32;
    let y1 = end.screen.y as i32;
    let dx = (x1 - x0).abs();
    let dy_absolute = (y1 - y0).abs();
    let steps = dx.max(dy_absolute) as u32;
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -dy_absolute;
    let sy = if y0 < y1 { 1 } else { -1 };
    let (first_step, last_step) =
        match depth_line_band_steps(y0, y1, dx, dy_absolute, band_y as i32, band_bottom) {
            Some(steps) => steps,
            None => return,
        };
    let (mut x0, mut y0, mut error) =
        depth_line_state_at_step(x0, y0, x1, y1, dx, dy_absolute, first_step);
    let mut step = first_step;
    loop {
        if x0 >= 0 && x0 < SCREEN_WIDTH as i32 && y0 >= band_y as i32 && y0 <= band_bottom {
            let key = if steps == 0 {
                start.inverse_depth
            } else {
                let start_weight = steps - step.min(steps);
                let end_weight = step.min(steps);
                ((start.inverse_depth as u32 * start_weight
                    + end.inverse_depth as u32 * end_weight)
                    / steps) as u16
            };
            let local_y = y0 as usize - band_y;
            let index = local_y * SCREEN_WIDTH + x0 as usize;
            if index < pixels.len()
                && index < depth.len()
                && key.saturating_add(SURFACE_GRID_DEPTH_TOLERANCE) >= depth[index]
            {
                pixels[index] = color;
            }
        }
        // Renderer-produced endpoints are tightly bounded, but retain a hard
        // stop for unexpected direct callers so a corrupt line can never hang
        // the application or watchdog.
        if step >= steps {
            break;
        }
        if step == last_step {
            break;
        }
        let twice_error = 2 * error;
        if twice_error >= dy {
            error += dy;
            x0 += sx;
        }
        if twice_error <= dx {
            error += dx;
            y0 += sy;
        }
        step = step.saturating_add(1);
    }
}

/// Determines the inclusive range of the original Bresenham step sequence
/// whose Y coordinate belongs to this band. The surface projector bounds
/// screen coordinates to roughly ±800, so the small integer products below
/// remain safely in `i32`. Out-of-contract direct callers retain the original
/// full traversal through the conservative fallback.
#[cfg(test)]
fn depth_line_band_steps(
    y0: i32,
    y1: i32,
    dx: i32,
    dy_absolute: i32,
    band_top: i32,
    band_bottom: i32,
) -> Option<(u32, u32)> {
    let steps = dx.max(dy_absolute);
    if steps == 0 {
        return if y0 >= band_top && y0 <= band_bottom {
            Some((0, 0))
        } else {
            None
        };
    }
    // `ScreenPoint` is public for tests, but renderer-produced points are
    // bounded by Projector::project_transformed. Avoid overflow if a direct
    // caller supplies arbitrary i16 endpoints.
    if dx > 2_048 || dy_absolute > 2_048 {
        return Some((0, steps as u32));
    }
    if dy_absolute == 0 {
        return if y0 >= band_top && y0 <= band_bottom {
            Some((0, steps as u32))
        } else {
            None
        };
    }

    let downward = y1 >= y0;
    let lower_progress = if downward {
        (band_top - y0).max(0)
    } else {
        (y0 - band_bottom).max(0)
    };
    let upper_progress = if downward {
        (band_bottom - y0).min(dy_absolute)
    } else {
        (y0 - band_top).min(dy_absolute)
    };
    if lower_progress > upper_progress || upper_progress < 0 {
        return None;
    }
    let lower_progress = lower_progress.min(dy_absolute);
    let upper_progress = upper_progress.max(0).min(dy_absolute);

    if dy_absolute > dx {
        return Some((lower_progress as u32, upper_progress as u32));
    }

    // For X-major Bresenham lines, Y has advanced by
    // floor((dy * step + dx / 2) / dx). Inverting that exact expression gives
    // the first/last global steps for this band without replaying the complete
    // line from its endpoint for every band it crosses.
    let first_numerator = lower_progress * dx - dx / 2;
    let first_step = if first_numerator <= 0 {
        0
    } else {
        (first_numerator + dy_absolute - 1) / dy_absolute
    };
    let last_numerator = (upper_progress + 1) * dx - dx / 2 - 1;
    let last_step = last_numerator / dy_absolute;
    let first_step = first_step.max(0).min(steps);
    let last_step = last_step.max(0).min(steps);
    if first_step > last_step {
        None
    } else {
        Some((first_step as u32, last_step as u32))
    }
}

/// Reconstructs the exact Bresenham point and error state after `step` global
/// steps. Continuing the ordinary loop from this state preserves both pixel
/// coverage and the existing global-step depth interpolation.
#[cfg(test)]
fn depth_line_state_at_step(
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    dx: i32,
    dy_absolute: i32,
    step: u32,
) -> (i32, i32, i32) {
    let step = step as i32;
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    if dx >= dy_absolute {
        let vertical_steps = (dy_absolute * step + dx / 2) / dx;
        (
            x0 + sx * step,
            y0 + sy * vertical_steps,
            dx - dy_absolute - step * dy_absolute + vertical_steps * dx,
        )
    } else {
        let horizontal_steps = (dx * step + dy_absolute / 2) / dy_absolute;
        (
            x0 + sx * horizontal_steps,
            y0 + sy * step,
            dx - dy_absolute + step * dx - horizontal_steps * dy_absolute,
        )
    }
}

#[cfg(test)]
/// Original whole-line band walk retained only as a regression oracle. The
/// optimized traversal must produce the same pixels for every individual band.
fn draw_depth_line_reference(
    pixels: &mut [Color],
    depth: &[u16],
    band_y: usize,
    start: SolidVertex,
    end: SolidVertex,
    color: Color,
) {
    if !start.is_visible() || !end.is_visible() {
        return;
    }
    let band_bottom = band_y as i32 + BAND_HEIGHT as i32 - 1;
    let min_y = (start.screen.y.min(end.screen.y)) as i32;
    let max_y = (start.screen.y.max(end.screen.y)) as i32;
    if max_y < band_y as i32 || min_y > band_bottom {
        return;
    }

    let mut x0 = start.screen.x as i32;
    let mut y0 = start.screen.y as i32;
    let x1 = end.screen.x as i32;
    let y1 = end.screen.y as i32;
    let dx = (x1 - x0).abs();
    let dy_absolute = (y1 - y0).abs();
    let steps = dx.max(dy_absolute) as u32;
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -dy_absolute;
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    let mut step = 0_u32;
    loop {
        if x0 >= 0 && x0 < SCREEN_WIDTH as i32 && y0 >= band_y as i32 && y0 <= band_bottom {
            let key = if steps == 0 {
                start.inverse_depth
            } else {
                let start_weight = steps - step.min(steps);
                let end_weight = step.min(steps);
                ((start.inverse_depth as u32 * start_weight
                    + end.inverse_depth as u32 * end_weight)
                    / steps) as u16
            };
            let local_y = y0 as usize - band_y;
            let index = local_y * SCREEN_WIDTH + x0 as usize;
            if index < pixels.len()
                && index < depth.len()
                && key.saturating_add(SURFACE_GRID_DEPTH_TOLERANCE) >= depth[index]
            {
                pixels[index] = color;
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice_error = 2 * error;
        if twice_error >= dy {
            error += dy;
            x0 += sx;
        }
        if twice_error <= dx {
            error += dx;
            y0 += sy;
        }
        step = step.saturating_add(1);
    }
}

fn edge_at_vertex(start: ScreenPoint, end: ScreenPoint, point: ScreenPoint) -> i32 {
    let delta_x = end.x as i32 - start.x as i32;
    let delta_y = end.y as i32 - start.y as i32;
    delta_x * (point.y as i32 - start.y as i32) - delta_y * (point.x as i32 - start.x as i32)
}

fn edge_at_pixel_center(start: ScreenPoint, end: ScreenPoint, pixel_x: i32, pixel_y: i32) -> i32 {
    let delta_x = end.x as i32 - start.x as i32;
    let delta_y = end.y as i32 - start.y as i32;
    let relative_x_twice = pixel_x * 2 + 1 - start.x as i32 * 2;
    let relative_y_twice = pixel_y * 2 + 1 - start.y as i32 * 2;
    delta_x * relative_y_twice - delta_y * relative_x_twice
}

fn minimum3(first: i32, second: i32, third: i32) -> i32 {
    first.min(second).min(third)
}

fn maximum3(first: i32, second: i32, third: i32) -> i32 {
    first.max(second).max(third)
}

fn encode_inverse_depth(depth: f32) -> u16 {
    if !depth.is_finite() || depth < SOLID_NEAR_DEPTH {
        return 0;
    }
    let normalized = SOLID_NEAR_DEPTH / depth;
    if !normalized.is_finite() || normalized <= 0.0 {
        0
    } else {
        let encoded = (normalized.min(1.0) * (DEPTH_KEY_MAX - 1.0)) as u16;
        encoded.saturating_add(1)
    }
}

fn shaded_surface_color(shade: u8) -> Color {
    Color {
        rgb565: SHADE_COLOR_LUT[shade as usize],
    }
}

#[cfg(test)]
fn shaded_surface_color_reference(shade: u8) -> Color {
    let red = ((PALETTE.solid_surface.rgb565 >> 11) & 0x1f) as u32;
    let green = ((PALETTE.solid_surface.rgb565 >> 5) & 0x3f) as u32;
    let blue = (PALETTE.solid_surface.rgb565 & 0x1f) as u32;
    let shade = shade as u32;
    Color {
        rgb565: (((red * shade / 255) as u16) << 11)
            | (((green * shade / 255) as u16) << 5)
            | (blue * shade / 255) as u16,
    }
}

fn draw_geometry_lines(
    pixels: &mut [Color],
    band_y: usize,
    geometry: &WorldGeometry,
    layer: LineLayer,
) {
    let mut index = 0;
    while index < geometry.line_count {
        if let Some(colored) = geometry.lines[index] {
            if colored.layer == layer {
                draw_line(
                    pixels,
                    band_y,
                    colored.line.start,
                    colored.line.end,
                    colored.color,
                );
            }
        }
        index += 1;
    }
}

fn draw_labels(pixels: &mut [Color], band_y: usize, geometry: &WorldGeometry, numeric: bool) {
    let mut index = 0;
    while index < geometry.label_count {
        if let Some(label) = geometry.labels[index] {
            if matches!(label.kind, LabelKind::Number(_)) == numeric {
                draw_label(pixels, band_y, label);
            }
        }
        index += 1;
    }
}

fn draw_label_backgrounds(pixels: &mut [Color], band_y: usize, geometry: &WorldGeometry) {
    let mut index = 0;
    while index < geometry.label_count {
        if let Some(label) = geometry.labels[index] {
            if matches!(label.kind, LabelKind::Number(_)) {
                if let Some(rect) = label_rect(label.anchor, label.kind) {
                    fill_band_rect(
                        pixels,
                        band_y,
                        expand_label_rect(rect, LABEL_PADDING),
                        PALETTE.background,
                    );
                }
            }
        }
        index += 1;
    }
}

fn draw_label(pixels: &mut [Color], band_y: usize, label: Label) {
    let mut buffer = [0_u8; 12];
    let text = label_text(label.kind, &mut buffer);
    let rect = match label_rect(label.anchor, label.kind) {
        Some(rect) => rect,
        None => return,
    };
    let mut character_index = 0;
    while character_index < text.len() && text[character_index] != 0 {
        let rows = match glyph(text[character_index]) {
            Some(rows) => rows,
            None => return,
        };
        let mut glyph_y = 0;
        while glyph_y < rows.len() {
            let screen_y = rect.top + glyph_y as i32;
            if screen_y >= band_y as i32 && screen_y < (band_y + BAND_HEIGHT) as i32 {
                let mut glyph_x = 0;
                while glyph_x < GLYPH_WIDTH {
                    if rows[glyph_y] & (1 << (GLYPH_WIDTH - 1 - glyph_x)) != 0 {
                        let screen_x = rect.left
                            + character_index as i32 * (GLYPH_WIDTH + GLYPH_SPACING)
                            + glyph_x;
                        if screen_x >= 0 && screen_x < SCREEN_WIDTH as i32 {
                            let local_y = screen_y as usize - band_y;
                            let pixel_index = local_y * SCREEN_WIDTH + screen_x as usize;
                            if pixel_index < pixels.len() {
                                pixels[pixel_index] = label.color;
                            }
                        }
                    }
                    glyph_x += 1;
                }
            }
            glyph_y += 1;
        }
        character_index += 1;
    }
}

fn fill_band_rect(pixels: &mut [Color], band_y: usize, rect: LabelRect, color: Color) {
    let top = rect.top.max(band_y as i32);
    let bottom = rect.bottom.min((band_y + BAND_HEIGHT - 1) as i32);
    if top > bottom {
        return;
    }
    let left = rect.left.max(0);
    let right = rect.right.min(SCREEN_WIDTH as i32 - 1);
    let mut y = top;
    while y <= bottom {
        let mut x = left;
        while x <= right {
            let local_y = y as usize - band_y;
            let pixel_index = local_y * SCREEN_WIDTH + x as usize;
            if pixel_index < pixels.len() {
                pixels[pixel_index] = color;
            }
            x += 1;
        }
        y += 1;
    }
}

fn draw_origin(pixels: &mut [Color], band_y: usize, origin: Option<ScreenPoint>) {
    if let Some(origin) = origin {
        draw_line(
            pixels,
            band_y,
            ScreenPoint {
                x: origin.x - 2,
                y: origin.y,
            },
            ScreenPoint {
                x: origin.x + 2,
                y: origin.y,
            },
            PALETTE.origin,
        );
        draw_line(
            pixels,
            band_y,
            ScreenPoint {
                x: origin.x,
                y: origin.y - 2,
            },
            ScreenPoint {
                x: origin.x,
                y: origin.y + 2,
            },
            PALETTE.origin,
        );
    }
}

fn label_text(kind: LabelKind, buffer: &mut [u8; 12]) -> &[u8] {
    match kind {
        LabelKind::AxisX => b"X\0",
        LabelKind::AxisY => b"Y\0",
        LabelKind::AxisZ => b"Z\0",
        LabelKind::Number(value) => format_tick(value, buffer),
    }
}

fn label_rect(anchor: ScreenPoint, kind: LabelKind) -> Option<LabelRect> {
    if !anchor.is_visible() {
        return None;
    }
    let mut buffer = [0_u8; 12];
    let text = label_text(kind, &mut buffer);
    let width = text_width(text)?;
    let (x_offset, y_offset) = match kind {
        LabelKind::Number(_) => (4, -9),
        LabelKind::AxisZ => (-11, -12),
        _ => (5, -12),
    };
    let rect = LabelRect {
        left: anchor.x as i32 + x_offset,
        top: anchor.y as i32 + y_offset,
        right: anchor.x as i32 + x_offset + width - 1,
        bottom: anchor.y as i32 + y_offset + GLYPH_HEIGHT - 1,
    };
    let padded = expand_label_rect(rect, LABEL_PADDING);
    if padded.left < 1
        || padded.right >= SCREEN_WIDTH as i32 - 1
        || padded.top < GRAPH_TOP as i32 + 1
        || padded.bottom >= SCREEN_HEIGHT as i32 - 1
    {
        None
    } else {
        Some(rect)
    }
}

fn text_width(text: &[u8]) -> Option<i32> {
    let mut characters = 0_i32;
    while characters < text.len() as i32 && text[characters as usize] != 0 {
        glyph(text[characters as usize])?;
        characters += 1;
    }
    if characters == 0 {
        None
    } else {
        Some(characters * GLYPH_WIDTH + (characters - 1) * GLYPH_SPACING)
    }
}

fn glyph(character: u8) -> Option<&'static [u8; 7]> {
    match character {
        b'0'..=b'9' => Some(&NUMERIC_GLYPHS[(character - b'0') as usize]),
        b'-' => Some(&NUMERIC_GLYPHS[10]),
        b'.' => Some(&NUMERIC_GLYPHS[11]),
        b'+' => Some(&NUMERIC_GLYPHS[12]),
        b'X' => Some(&AXIS_X_GLYPH),
        b'Y' => Some(&AXIS_Y_GLYPH),
        b'Z' => Some(&AXIS_Z_GLYPH),
        _ => None,
    }
}

fn expand_label_rect(rect: LabelRect, amount: i32) -> LabelRect {
    LabelRect {
        left: rect.left - amount,
        top: rect.top - amount,
        right: rect.right + amount,
        bottom: rect.bottom + amount,
    }
}

fn label_rects_overlap(first: LabelRect, second: LabelRect, separation: i32) -> bool {
    let first = expand_label_rect(first, separation);
    first.left <= second.right
        && first.right >= second.left
        && first.top <= second.bottom
        && first.bottom >= second.top
}

fn surface_distance_squared(rect: LabelRect, projected_surface: ProjectedSurface<'_>) -> i32 {
    let mut minimum = i32::MAX;
    let mut row = 0;
    while row < ROWS {
        let mut column = 0;
        while column < COLUMNS {
            let point = projected_surface.screen(row, column);
            if point.is_visible() {
                let x = point.x as i32;
                let y = point.y as i32;
                let dx = if x < rect.left {
                    rect.left - x
                } else if x > rect.right {
                    x - rect.right
                } else {
                    0
                };
                let dy = if y < rect.top {
                    rect.top - y
                } else if y > rect.bottom {
                    y - rect.bottom
                } else {
                    0
                };
                let distance = dx * dx + dy * dy;
                if distance < minimum {
                    minimum = distance;
                }
            }
            column += 1;
        }
        row += 1;
    }
    minimum
}

fn format_tick(value: f32, buffer: &mut [u8; 12]) -> &[u8] {
    let mut index = 0;
    let mut absolute = value;
    if absolute < 0.0 {
        buffer[index] = b'-';
        index += 1;
        absolute = -absolute;
    }
    let integer = absolute as u32;
    let mut divisor = 1_u32;
    while divisor <= integer / 10 {
        divisor *= 10;
    }
    loop {
        if index + 1 >= buffer.len() {
            break;
        }
        buffer[index] = b'0' + (integer / divisor % 10) as u8;
        index += 1;
        if divisor == 1 {
            break;
        }
        divisor /= 10;
    }
    let hundredths = ((absolute - integer as f32) * 100.0 + 0.5) as u32;
    if hundredths > 0 && index + 2 < buffer.len() {
        buffer[index] = b'.';
        buffer[index + 1] = b'0' + (hundredths / 10 % 10) as u8;
        index += 2;
        if hundredths % 10 != 0 && index + 1 < buffer.len() {
            buffer[index] = b'0' + (hundredths % 10) as u8;
            index += 1;
        }
    }
    buffer[index] = 0;
    &buffer[..index + 1]
}

fn z_axis_range(surface_min: f32, surface_max: f32, has_height: bool) -> (f32, f32) {
    if !has_height {
        return (-1.0, 1.0);
    }
    let min = if surface_min < 0.0 { surface_min } else { 0.0 };
    let max = if surface_max > 0.0 { surface_max } else { 0.0 };
    if min == max {
        (min - 1.0, max + 1.0)
    } else {
        (min, max)
    }
}

fn domain_tick_size(domain: Domain) -> f32 {
    let x_span = (domain.x_max - domain.x_min).abs();
    let y_span = (domain.y_max - domain.y_min).abs();
    let span = if x_span > y_span { x_span } else { y_span };
    if span.is_finite() && span > 0.0 {
        span * 0.025
    } else {
        0.1
    }
}

fn visible_point(point: ScreenPoint) -> Option<ScreenPoint> {
    if point.is_visible() {
        Some(point)
    } else {
        None
    }
}

fn draw_line(
    pixels: &mut [Color],
    band_y: usize,
    start: ScreenPoint,
    end: ScreenPoint,
    color: Color,
) {
    if !start.is_visible() || !end.is_visible() {
        return;
    }
    let band_bottom = band_y as i32 + BAND_HEIGHT as i32 - 1;
    let min_y = if start.y < end.y { start.y } else { end.y } as i32;
    let max_y = if start.y > end.y { start.y } else { end.y } as i32;
    if max_y < band_y as i32 || min_y > band_bottom {
        return;
    }

    let mut x0 = start.x as i32;
    let mut y0 = start.y as i32;
    let x1 = end.x as i32;
    let y1 = end.y as i32;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        if x0 >= 0 && x0 < SCREEN_WIDTH as i32 && y0 >= band_y as i32 && y0 <= band_bottom {
            let local_y = y0 as usize - band_y;
            let index = local_y * SCREEN_WIDTH + x0 as usize;
            if index < pixels.len() {
                pixels[index] = color;
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice_error = 2 * error;
        if twice_error >= dy {
            error += dy;
            x0 += sx;
        }
        if twice_error <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::CompiledExpression;

    fn empty_surface() -> [[ScreenPoint; COLUMNS]; ROWS] {
        [[ScreenPoint::INVALID; COLUMNS]; ROWS]
    }

    fn identity_projector() -> Projector {
        Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: 8.0,
            target_x: 0.0,
            target_y: 0.0,
            target_z: 0.0,
            focal_length: 235.0,
        }
        .projector()
    }

    #[test]
    fn rejects_label_anchors_behind_camera() {
        let mut geometry = WorldGeometry::new();
        let projected = identity_projector().project_with_depth(Point3 {
            x: 0.0,
            y: 9.0,
            z: 0.0,
        });
        assert!(!geometry.add_label(projected, LabelKind::AxisY, PALETTE.y_axis));
        assert_eq!(geometry.label_count, 0);
    }

    #[test]
    fn rejects_labels_outside_viewport_and_over_header() {
        assert!(label_rect(ScreenPoint { x: -5, y: 100 }, LabelKind::AxisX).is_none());
        assert!(label_rect(
            ScreenPoint {
                x: 100,
                y: GRAPH_TOP as i16 + 5
            },
            LabelKind::AxisX
        )
        .is_none());
        assert!(label_rect(ScreenPoint { x: 318, y: 100 }, LabelKind::AxisX).is_none());
    }

    #[test]
    fn numeric_label_selection_is_conservative() {
        let surface = empty_surface();
        let geometry = build_world_geometry(
            &Camera::new().projector(),
            Domain::DEFAULT,
            -1.0,
            1.0,
            true,
            ProjectedSurface::Wireframe(&surface),
            GraphOptions::DEFAULT,
        );
        let mut numeric = 0;
        let mut index = 0;
        while index < geometry.label_count {
            if matches!(
                geometry.labels[index],
                Some(Label {
                    kind: LabelKind::Number(_),
                    ..
                })
            ) {
                numeric += 1;
            }
            index += 1;
        }
        assert!(numeric <= MAX_NUMERIC_LABELS_PER_AXIS * 3);
        assert!(geometry.label_count <= MAX_LABELS);
    }

    #[test]
    fn identical_camera_state_generates_identical_labels() {
        let camera = Camera::new();
        let surface = empty_surface();
        let first = build_world_geometry(
            &camera.projector(),
            Domain::DEFAULT,
            -1.0,
            1.0,
            true,
            ProjectedSurface::Wireframe(&surface),
            GraphOptions::DEFAULT,
        );
        let second = build_world_geometry(
            &camera.projector(),
            Domain::DEFAULT,
            -1.0,
            1.0,
            true,
            ProjectedSurface::Wireframe(&surface),
            GraphOptions::DEFAULT,
        );
        assert_eq!(first.label_count, second.label_count);
        assert_eq!(first.labels, second.labels);
    }

    #[test]
    fn duplicate_labels_are_not_generated() {
        let mut geometry = WorldGeometry::new();
        let projected = Some(ProjectedPoint {
            screen: ScreenPoint { x: 100, y: 100 },
            depth: 8.0,
        });
        assert!(geometry.add_label(projected, LabelKind::AxisX, PALETTE.x_axis));
        assert!(!geometry.add_label(projected, LabelKind::AxisX, PALETTE.x_axis));
        assert_eq!(geometry.label_count, 1);
    }

    #[test]
    fn axes_replace_expendable_grid_lines_when_geometry_capacity_is_full() {
        let line = Some(ProjectedLine {
            start: ScreenPoint { x: 20, y: 40 },
            end: ScreenPoint { x: 40, y: 40 },
        });
        let mut geometry = WorldGeometry::new();
        let mut index = 0;
        while index < MAX_WORLD_LINES {
            geometry.add_line(line, PALETTE.grid, LineLayer::Grid);
            index += 1;
        }

        geometry.add_line(line, PALETTE.x_axis, LineLayer::Axis);
        assert_eq!(geometry.line_count, MAX_WORLD_LINES);

        let mut axes = 0;
        let mut grids = 0;
        index = 0;
        while index < geometry.line_count {
            match geometry.lines[index] {
                Some(ColoredLine {
                    layer: LineLayer::Axis,
                    ..
                }) => axes += 1,
                Some(ColoredLine {
                    layer: LineLayer::Grid,
                    ..
                }) => grids += 1,
                _ => {}
            }
            index += 1;
        }
        assert_eq!(axes, 1);
        assert_eq!(grids, MAX_WORLD_LINES - 1);
    }

    #[test]
    fn font_glyphs_are_five_by_seven() {
        assert_eq!(GLYPH_WIDTH, 5);
        assert_eq!(GLYPH_HEIGHT, 7);
        for glyph in NUMERIC_GLYPHS {
            assert_eq!(glyph.len(), GLYPH_HEIGHT as usize);
            for row in glyph {
                assert!(row <= 0b11111);
            }
        }
    }

    #[test]
    fn every_required_numeric_glyph_is_present() {
        for character in b'0'..=b'9' {
            assert!(glyph(character).is_some());
        }
        assert!(glyph(b'-').is_some());
        assert!(glyph(b'.').is_some());
        assert!(glyph(b'+').is_some());
    }

    #[test]
    fn text_width_accounts_for_spacing_and_punctuation() {
        assert_eq!(text_width(b"7\0"), Some(5));
        assert_eq!(text_width(b"12\0"), Some(11));
        assert_eq!(text_width(b"-3\0"), Some(11));
        assert_eq!(text_width(b"0.5\0"), Some(17));
    }

    #[test]
    fn overlapping_label_boxes_are_rejected() {
        let mut geometry = WorldGeometry::new();
        let first = Some(ProjectedPoint {
            screen: ScreenPoint { x: 100, y: 100 },
            depth: 8.0,
        });
        let overlapping = Some(ProjectedPoint {
            screen: ScreenPoint { x: 106, y: 100 },
            depth: 8.0,
        });
        assert!(geometry.add_label(first, LabelKind::Number(-2.0), PALETTE.text));
        assert!(!geometry.add_label(overlapping, LabelKind::Number(2.0), PALETTE.text));
    }

    #[test]
    fn candidate_selection_limits_each_axis_to_three_labels() {
        let mut geometry = WorldGeometry::new();
        let surface = empty_surface();
        add_x_ticks(
            &mut geometry,
            &Camera::new().projector(),
            Domain::DEFAULT,
            domain_tick_size(Domain::DEFAULT),
            ProjectedSurface::Wireframe(&surface),
            GraphOptions::DEFAULT,
        );
        let mut numeric = 0;
        let mut index = 0;
        while index < geometry.label_count {
            if matches!(
                geometry.labels[index],
                Some(Label {
                    kind: LabelKind::Number(_),
                    ..
                })
            ) {
                numeric += 1;
            }
            index += 1;
        }
        assert!(numeric <= MAX_NUMERIC_LABELS_PER_AXIS);
    }

    #[test]
    fn default_surface_keeps_a_useful_bounded_label_set() {
        let function = CompiledExpression::compile("sin(x) * cos(y)").expect("expression");
        let projector = Camera::new().projector();
        let mut projected = empty_surface();
        let mut z_min = 0.0_f32;
        let mut z_max = 0.0_f32;
        let mut row = 0;
        while row < ROWS {
            let mut column = 0;
            while column < COLUMNS {
                let point = crate::surface::point(Domain::DEFAULT, column, row, &function);
                if point.z < z_min {
                    z_min = point.z;
                }
                if point.z > z_max {
                    z_max = point.z;
                }
                projected[row][column] = projector.project(point);
                column += 1;
            }
            row += 1;
        }
        let geometry = build_world_geometry(
            &projector,
            Domain::DEFAULT,
            z_min,
            z_max,
            true,
            ProjectedSurface::Wireframe(&projected),
            GraphOptions::DEFAULT,
        );
        let mut numeric = 0;
        let mut axes = 0;
        let mut index = 0;
        while index < geometry.label_count {
            match geometry.labels[index] {
                Some(Label {
                    kind: LabelKind::Number(_),
                    ..
                }) => numeric += 1,
                Some(_) => axes += 1,
                None => {}
            }
            index += 1;
        }
        assert!(numeric >= 1);
        assert!(numeric <= MAX_NUMERIC_LABELS_PER_AXIS * 3);
        assert_eq!(axes, 3);
    }

    fn solid_vertex(x: i16, y: i16, inverse_depth: u16) -> SolidVertex {
        SolidVertex {
            screen: ScreenPoint { x, y },
            inverse_depth,
        }
    }

    #[test]
    fn compact_solid_vertex_and_depth_encoding_are_bounded() {
        assert_eq!(core::mem::size_of::<SolidVertex>(), 6);
        assert_eq!(
            core::mem::size_of::<[[ScreenPoint; COLUMNS]; ROWS]>(),
            1_900
        );
        assert_eq!(
            core::mem::size_of::<[[SolidVertex; COLUMNS]; ROWS]>(),
            2_850
        );
        assert_eq!(
            core::mem::size_of::<[Color; SCREEN_WIDTH * BAND_HEIGHT]>(),
            5_120
        );
        assert_eq!(
            core::mem::size_of::<[u16; SCREEN_WIDTH * BAND_HEIGHT]>(),
            5_120
        );
        assert_eq!(core::mem::size_of::<TriangleShades>(), 864);
        assert_eq!(encode_inverse_depth(1.0), 0);
        let near = encode_inverse_depth(SOLID_NEAR_DEPTH);
        let middle = encode_inverse_depth(8.0);
        let far = encode_inverse_depth(20.0);
        assert!(near > middle);
        assert!(middle > far);
        assert!(far > 0);
        assert_eq!(encode_inverse_depth(f32::NAN), 0);
        assert_eq!(encode_inverse_depth(f32::INFINITY), 0);
    }

    #[test]
    fn triangle_depth_is_independent_of_traversal_order() {
        let near = [
            solid_vertex(20, 26, 50_000),
            solid_vertex(60, 26, 50_000),
            solid_vertex(20, 31, 50_000),
        ];
        let far = [
            solid_vertex(20, 26, 10_000),
            solid_vertex(60, 26, 10_000),
            solid_vertex(20, 31, 10_000),
        ];
        let near_color = Color { rgb565: 0xf800 };
        let far_color = Color { rgb565: 0x07e0 };
        let mut first_pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
        let mut first_depth = [0_u16; SCREEN_WIDTH * BAND_HEIGHT];
        draw_triangle_band(&mut first_pixels, &mut first_depth, 24, far, far_color);
        draw_triangle_band(&mut first_pixels, &mut first_depth, 24, near, near_color);

        let mut second_pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
        let mut second_depth = [0_u16; SCREEN_WIDTH * BAND_HEIGHT];
        draw_triangle_band(&mut second_pixels, &mut second_depth, 24, near, near_color);
        draw_triangle_band(&mut second_pixels, &mut second_depth, 24, far, far_color);

        assert_eq!(first_depth, second_depth);
        assert_eq!(first_pixels, second_pixels);
        assert!(first_pixels.iter().any(|pixel| *pixel == near_color));
        assert!(!first_pixels.iter().any(|pixel| *pixel == far_color));
    }

    #[test]
    fn equal_depth_uses_a_stable_first_writer_tie_policy() {
        let triangle = [
            solid_vertex(20, 26, 25_000),
            solid_vertex(60, 26, 25_000),
            solid_vertex(20, 31, 25_000),
        ];
        let first_color = Color { rgb565: 0xf800 };
        let second_color = Color { rgb565: 0x07e0 };
        let mut pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
        let mut depth = [0_u16; SCREEN_WIDTH * BAND_HEIGHT];
        draw_triangle_band(&mut pixels, &mut depth, 24, triangle, first_color);
        let after_first = pixels;
        draw_triangle_band(&mut pixels, &mut depth, 24, triangle, second_color);
        assert_eq!(pixels, after_first);
    }

    #[test]
    fn incremental_depth_changes_monotonically_across_a_triangle() {
        let triangle = [
            solid_vertex(20, 26, 50_000),
            solid_vertex(100, 26, 10_000),
            solid_vertex(20, 31, 50_000),
        ];
        let mut pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
        let mut depth = [0_u16; SCREEN_WIDTH * BAND_HEIGHT];
        draw_triangle_band(
            &mut pixels,
            &mut depth,
            24,
            triangle,
            Color { rgb565: 0x1234 },
        );
        let left = depth[(27 - 24) * SCREEN_WIDTH + 25];
        let middle = depth[(27 - 24) * SCREEN_WIDTH + 45];
        let right = depth[(27 - 24) * SCREEN_WIDTH + 65];
        assert!(left > middle);
        assert!(middle > right);
        assert!(right > 0);
    }

    #[test]
    fn triangle_band_clipping_and_invalid_rejection_are_safe() {
        let mut pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
        let mut depth = [0_u16; SCREEN_WIDTH * BAND_HEIGHT];
        let crossing = [
            solid_vertex(-20, 20, 30_000),
            solid_vertex(40, 27, 30_000),
            solid_vertex(10, 40, 30_000),
        ];
        draw_triangle_band(
            &mut pixels,
            &mut depth,
            24,
            crossing,
            Color { rgb565: 0x1234 },
        );
        assert!(depth.iter().any(|value| *value != 0));

        let before = pixels;
        let invalid = [SolidVertex::INVALID, crossing[1], crossing[2]];
        draw_triangle_band(
            &mut pixels,
            &mut depth,
            24,
            invalid,
            Color { rgb565: 0xffff },
        );
        assert_eq!(pixels, before);

        let outside = [
            solid_vertex(10, 5, 30_000),
            solid_vertex(40, 5, 30_000),
            solid_vertex(10, 10, 30_000),
        ];
        draw_triangle_band(
            &mut pixels,
            &mut depth,
            24,
            outside,
            Color { rgb565: 0xffff },
        );
        assert_eq!(pixels, before);
    }

    #[test]
    fn solid_grid_line_obeys_filled_depth() {
        let mut hidden_pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
        let hidden_depth = [30_000_u16; SCREEN_WIDTH * BAND_HEIGHT];
        draw_depth_line(
            &mut hidden_pixels,
            &hidden_depth,
            24,
            solid_vertex(10, 27, 10_000),
            solid_vertex(30, 27, 10_000),
            PALETTE.grid,
        );
        assert!(hidden_pixels
            .iter()
            .all(|pixel| *pixel == PALETTE.background));

        let mut visible_pixels = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
        draw_depth_line(
            &mut visible_pixels,
            &hidden_depth,
            24,
            solid_vertex(10, 27, 40_000),
            solid_vertex(30, 27, 40_000),
            PALETTE.grid,
        );
        assert!(visible_pixels.iter().any(|pixel| *pixel == PALETTE.grid));
    }

    fn assert_depth_line_matches_reference(start: SolidVertex, end: SolidVertex, fill_depth: u16) {
        let mut band_y = GRAPH_TOP;
        while band_y < SCREEN_HEIGHT {
            let mut optimized = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
            let mut reference = [PALETTE.background; SCREEN_WIDTH * BAND_HEIGHT];
            let depth = [fill_depth; SCREEN_WIDTH * BAND_HEIGHT];
            draw_depth_line(&mut optimized, &depth, band_y, start, end, PALETTE.grid);
            draw_depth_line_reference(&mut reference, &depth, band_y, start, end, PALETTE.grid);
            assert_eq!(optimized, reference, "band {band_y}");
            band_y += BAND_HEIGHT;
        }
    }

    #[test]
    fn clipped_depth_grid_lines_match_the_original_bresenham_path() {
        // Exercise both major-axis cases, both directions, band boundaries,
        // and positive/negative reciprocal-depth gradients. Every optimized
        // band must match the old complete-line traversal exactly.
        let cases = [
            (solid_vertex(8, 24, 12_000), solid_vertex(300, 40, 58_000)),
            (solid_vertex(300, 40, 58_000), solid_vertex(8, 24, 12_000)),
            (solid_vertex(160, 24, 50_000), solid_vertex(180, 238, 8_000)),
            (solid_vertex(180, 238, 8_000), solid_vertex(160, 24, 50_000)),
            (solid_vertex(10, 31, 24_000), solid_vertex(310, 31, 40_000)),
            (
                solid_vertex(111, 24, 40_000),
                solid_vertex(111, 239, 24_000),
            ),
            (solid_vertex(-20, 20, 60_000), solid_vertex(340, 220, 8_000)),
        ];
        for (start, end) in cases {
            assert_depth_line_matches_reference(start, end, 0);
            // A nonzero completed-fill depth exercises both visible and hidden
            // portions under the unchanged overlay tolerance policy.
            assert_depth_line_matches_reference(start, end, 30_000);
        }
    }

    #[test]
    fn triangle_band_overlap_rejects_only_nonintersecting_bands() {
        let first = solid_vertex(10, 31, 20_000);
        let second = solid_vertex(50, 34, 20_000);
        let third = solid_vertex(20, 39, 20_000);
        assert!(triangle_intersects_band(first, second, third, 24));
        assert!(triangle_intersects_band(first, second, third, 32));
        assert!(!triangle_intersects_band(first, second, third, 40));
    }

    #[test]
    fn solid_grid_topology_visits_each_unique_edge_once() {
        assert_eq!(SURFACE_GRID_EDGE_COUNT, 906);
        assert_eq!(ROWS * (COLUMNS - 1), 456);
        assert_eq!(COLUMNS * (ROWS - 1), 450);
    }

    #[test]
    fn solid_grid_does_not_bridge_invalid_triangle_regions() {
        let mut shades = [[[0_u8; TRIANGLES_PER_CELL]; COLUMNS - 1]; ROWS - 1];
        assert!(!horizontal_surface_edge_is_valid(&shades, 1, 1));
        assert!(!vertical_surface_edge_is_valid(&shades, 1, 1));

        shades[1][1][0] = 100;
        assert!(horizontal_surface_edge_is_valid(&shades, 1, 1));
        shades[1][1][0] = 0;
        shades[1][1][1] = 100;
        assert!(vertical_surface_edge_is_valid(&shades, 1, 1));

        assert!(!horizontal_surface_edge_is_valid(&shades, ROWS, 0));
        assert!(!vertical_surface_edge_is_valid(&shades, 0, COLUMNS));
    }

    #[test]
    fn rgb565_triangle_lighting_stays_within_the_base_channels() {
        let darkest = shaded_surface_color(1).rgb565;
        let brightest = shaded_surface_color(255).rgb565;
        assert_eq!(brightest, PALETTE.solid_surface.rgb565);
        assert_ne!(brightest, darkest);
        assert!((darkest >> 11) <= (brightest >> 11));
        assert!(((darkest >> 5) & 0x3f) <= ((brightest >> 5) & 0x3f));
        assert!((darkest & 0x1f) <= (brightest & 0x1f));
    }

    #[test]
    fn shade_lookup_is_exactly_the_previous_rgb565_calculation() {
        let mut shade = 0_u16;
        while shade <= u8::MAX as u16 {
            let shade_u8 = shade as u8;
            assert_eq!(
                shaded_surface_color(shade_u8),
                shaded_surface_color_reference(shade_u8),
                "shade {shade_u8}"
            );
            if shade_u8 == u8::MAX {
                break;
            }
            shade += 1;
        }
    }

    #[test]
    fn graph_options_remove_coordinate_geometry_without_touching_surface() {
        let hidden = GraphOptions {
            rendering_mode: RenderingMode::Wireframe,
            show_grid: false,
            show_axes: false,
            show_ticks: false,
            show_labels: false,
            show_performance: false,
        };
        let geometry = build_world_geometry(
            &Camera::new().projector(),
            Domain::DEFAULT,
            -1.0,
            1.0,
            true,
            ProjectedSurface::Wireframe(&empty_surface()),
            hidden,
        );
        assert_eq!(geometry.line_count, 0);
        assert_eq!(geometry.label_count, 0);
        assert_eq!(geometry.origin, None);
    }
}
