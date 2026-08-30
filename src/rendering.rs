//! Allocation-free wireframe graph renderer for the 320×240 RGB565 display.
//!
//! The 24-pixel header leaves a 320×216 graph viewport. Rendering walks that
//! viewport in 320×8 bands (2,560 pixels / 5,120 bytes), producing exactly 27
//! display transfers without a 153,600-byte full-screen framebuffer or depth
//! buffer. The sampled surface is projected once into a 25×19 screen-point cache.
//!
//! Composition order is deliberately repeated inside every band: clear, grid,
//! axes, label backgrounds, numeric glyphs, surface wireframe, ticks/origin,
//! axis glyphs, then push. Graph labels must remain in this buffer. Drawing text
//! directly to the display between band transfers can expose stale positions or
//! overwrite labels with later bands, reintroducing the hardware flashing bug.

use crate::camera::{Camera, ProjectedLine, ProjectedPoint, Projector, ScreenPoint};
use crate::eadk::{self, Color, Rect};
use crate::graph::{self, AxisVisibility, TickGenerator, PALETTE};
use crate::surface::{Domain, Point3, SurfaceGrid, COLUMNS, ROWS};

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
        if self.line_count >= MAX_WORLD_LINES {
            return;
        }
        if let Some(line) = line {
            self.lines[self.line_count] = Some(ColoredLine { line, color, layer });
            self.line_count += 1;
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
pub fn render(camera: &Camera, domain: Domain, surface: &SurfaceGrid) {
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

    let geometry = build_world_geometry(&projector, domain, z_min, z_max, has_height, &projected);

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
        draw_surface(&mut pixels, band_y, &projected);
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

fn build_world_geometry(
    projector: &Projector,
    domain: Domain,
    surface_z_min: f32,
    surface_z_max: f32,
    has_height: bool,
    projected_surface: &[[ScreenPoint; COLUMNS]; ROWS],
) -> WorldGeometry {
    let mut geometry = WorldGeometry::new();
    let visibility = graph::axes_for_domain(domain);
    let (z_min, z_max) = z_axis_range(surface_z_min, surface_z_max, has_height);
    add_grid(&mut geometry, projector, domain);
    add_axes(
        &mut geometry,
        projector,
        domain,
        z_min,
        z_max,
        visibility,
        projected_surface,
    );
    if visibility.z {
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
    projected_surface: &[[ScreenPoint; COLUMNS]; ROWS],
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
        add_x_ticks(geometry, projector, domain, tick_size, projected_surface);
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
        add_y_ticks(geometry, projector, domain, tick_size, projected_surface);
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
        add_z_ticks(
            geometry,
            projector,
            z_min,
            z_max,
            tick_size,
            projected_surface,
        );
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

fn add_x_ticks(
    geometry: &mut WorldGeometry,
    projector: &Projector,
    domain: Domain,
    tick_size: f32,
    projected_surface: &[[ScreenPoint; COLUMNS]; ROWS],
) {
    let mut ticks = TickGenerator::new(domain.x_min, domain.x_max);
    let mut candidates = NumericCandidates::new();
    while let Some(x) = ticks.next() {
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
        if x != 0.0 {
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
    select_numeric_labels(geometry, &mut candidates);
}

fn add_y_ticks(
    geometry: &mut WorldGeometry,
    projector: &Projector,
    domain: Domain,
    tick_size: f32,
    projected_surface: &[[ScreenPoint; COLUMNS]; ROWS],
) {
    let mut ticks = TickGenerator::new(domain.y_min, domain.y_max);
    let mut candidates = NumericCandidates::new();
    while let Some(y) = ticks.next() {
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
        if y != 0.0 {
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
    select_numeric_labels(geometry, &mut candidates);
}

fn add_z_ticks(
    geometry: &mut WorldGeometry,
    projector: &Projector,
    z_min: f32,
    z_max: f32,
    tick_size: f32,
    projected_surface: &[[ScreenPoint; COLUMNS]; ROWS],
) {
    let mut ticks = TickGenerator::new(z_min, z_max);
    let mut candidates = NumericCandidates::new();
    while let Some(z) = ticks.next() {
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
        if z != 0.0 {
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
    select_numeric_labels(geometry, &mut candidates);
}

fn consider_numeric_candidate(
    candidates: &mut NumericCandidates,
    projected: Option<ProjectedPoint>,
    value: f32,
    projected_surface: &[[ScreenPoint; COLUMNS]; ROWS],
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
    projected_surface: &[[ScreenPoint; COLUMNS]; ROWS],
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

fn draw_surface(pixels: &mut [Color], band_y: usize, projected: &[[ScreenPoint; COLUMNS]; ROWS]) {
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

fn surface_distance_squared(
    rect: LabelRect,
    projected_surface: &[[ScreenPoint; COLUMNS]; ROWS],
) -> i32 {
    let mut minimum = i32::MAX;
    let mut row = 0;
    while row < ROWS {
        let mut column = 0;
        while column < COLUMNS {
            let point = projected_surface[row][column];
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
            &surface,
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
            &surface,
        );
        let second = build_world_geometry(
            &camera.projector(),
            Domain::DEFAULT,
            -1.0,
            1.0,
            true,
            &surface,
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
            &surface,
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
        let geometry =
            build_world_geometry(&projector, Domain::DEFAULT, z_min, z_max, true, &projected);
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
}
