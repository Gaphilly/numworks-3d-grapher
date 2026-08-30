//! Orbit-camera state, view-space transformation, near-plane clipping, and projection.
//!
//! World coordinates use `x` and `y` for the mathematical domain and `z` for
//! height. At zero yaw and pitch, world `+x` points right on screen, world `+z`
//! points up, and the camera sits on the target's `+y` side looking toward the
//! target. Positive yaw orbits around world `+z`; positive pitch raises the
//! viewpoint. Screen `y` increases downward, so projection negates view-space
//! `z`.
//!
//! The camera is deliberately represented by an orbit target and distance, not
//! a general matrix. This keeps the hot projection path small and allocation-free
//! while still distinguishing orbit, truck, pedestal, dolly, and perspective.

use crate::math;
use crate::surface::Point3;

const SCREEN_CENTER_X: f32 = 160.0;
const SCREEN_CENTER_Y: f32 = 120.0;
const NEAR_DEPTH: f32 = 1.05;

/// Closest permitted orbit distance. This leaves useful clearance from the near
/// plane for the default `[-pi, pi]` domain even at steep camera angles.
pub const MIN_DISTANCE: f32 = 5.5;
/// Furthest permitted orbit distance; larger values make the graph impractically small.
pub const MAX_DISTANCE: f32 = 20.0;
/// Limits scene panning so an accidentally held key cannot lose the graph indefinitely.
const MAX_TARGET_OFFSET: f32 = 8.0;
const MIN_FOCAL_LENGTH: f32 = 150.0;
const MAX_FOCAL_LENGTH: f32 = 340.0;

/// A signed display coordinate. `INVALID` is a sentinel used instead of an
/// `Option` in the fixed-size projected surface cache.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPoint {
    pub x: i16,
    pub y: i16,
}

/// A projected anchor plus its positive camera-space distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectedPoint {
    pub screen: ScreenPoint,
    pub depth: f32,
}

/// A near-plane-clipped line ready for 2D band rasterization.
#[derive(Clone, Copy)]
pub struct ProjectedLine {
    pub start: ScreenPoint,
    pub end: ScreenPoint,
}

impl ScreenPoint {
    /// Sentinel guaranteed not to be produced by the bounded projection.
    pub const INVALID: ScreenPoint = ScreenPoint { x: i16::MIN, y: 0 };

    /// Returns whether this point contains a usable projected coordinate.
    pub fn is_visible(self) -> bool {
        self.x != i16::MIN
    }
}

/// Compact orbit camera used by the graph view.
///
/// `distance` is a real world/view-space distance. Changing it is a dolly: the
/// camera moves along its viewing direction while the target remains fixed.
/// `focal_length` independently controls perspective/FOV and is measured in
/// pixels. Target translation moves both the orbit center and implied camera,
/// leaving graph vertices stationary in world coordinates.
pub struct Camera {
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) distance: f32,
    pub(crate) target_x: f32,
    pub(crate) target_y: f32,
    pub(crate) target_z: f32,
    pub(crate) focal_length: f32,
}

impl Camera {
    /// Creates the established three-quarter view of the origin.
    pub const fn new() -> Camera {
        Camera {
            yaw: -0.65,
            pitch: 0.65,
            distance: 8.0,
            target_x: 0.0,
            target_y: 0.0,
            target_z: 0.0,
            focal_length: 235.0,
        }
    }

    /// Orbits around the current target. Pitch is clamped before the camera can
    /// flip over; non-finite input is ignored.
    pub fn orbit(&mut self, yaw_delta: f32, pitch_delta: f32) {
        if yaw_delta.is_finite() {
            self.yaw += yaw_delta;
            // Bounding yaw avoids losing precision after very long key holds.
            if self.yaw > core::f32::consts::PI {
                self.yaw -= 2.0 * core::f32::consts::PI;
            } else if self.yaw < -core::f32::consts::PI {
                self.yaw += 2.0 * core::f32::consts::PI;
            }
        }
        if pitch_delta.is_finite() {
            self.pitch = clamp(self.pitch + pitch_delta, -1.35, 1.35);
        }
    }

    /// Moves the orbit target and camera laterally along the camera's horizontal
    /// right vector. A positive amount trucks right in view space.
    pub fn truck(&mut self, amount: f32) {
        if !amount.is_finite() {
            return;
        }
        let (sin_yaw, cos_yaw) = math::sin_cos(self.yaw);
        self.target_x = clamp(
            self.target_x + cos_yaw * amount,
            -MAX_TARGET_OFFSET,
            MAX_TARGET_OFFSET,
        );
        self.target_y = clamp(
            self.target_y - sin_yaw * amount,
            -MAX_TARGET_OFFSET,
            MAX_TARGET_OFFSET,
        );
    }

    /// Moves the orbit target and camera along world `z`. This predictable world-
    /// vertical behavior is preferable to pitching the pedestal direction.
    pub fn pedestal(&mut self, amount: f32) {
        if amount.is_finite() {
            self.target_z = clamp(
                self.target_z + amount,
                -MAX_TARGET_OFFSET,
                MAX_TARGET_OFFSET,
            );
        }
    }

    /// Changes camera-to-target distance without changing perspective FOV.
    /// Negative values dolly toward the target; positive values dolly away.
    pub fn dolly(&mut self, amount: f32) {
        if amount.is_finite() {
            self.distance = clamp(self.distance + amount, MIN_DISTANCE, MAX_DISTANCE);
        }
    }

    /// Changes perspective focal length while keeping camera position fixed.
    /// This preserves the old optical-zoom capability under a separate binding.
    pub fn adjust_focal_length(&mut self, amount: f32) {
        if amount.is_finite() {
            self.focal_length = clamp(
                self.focal_length + amount,
                MIN_FOCAL_LENGTH,
                MAX_FOCAL_LENGTH,
            );
        }
    }

    /// Precomputes trigonometric terms once per graph redraw. Surface vertices
    /// then need only multiplies/adds in the projection loop.
    pub fn projector(&self) -> Projector {
        let (sin_yaw, cos_yaw) = math::sin_cos(self.yaw);
        let (sin_pitch, cos_pitch) = math::sin_cos(self.pitch);
        Projector {
            sin_yaw,
            cos_yaw,
            sin_pitch,
            cos_pitch,
            distance: self.distance,
            target_x: self.target_x,
            target_y: self.target_y,
            target_z: self.target_z,
            focal_length: self.focal_length,
        }
    }
}

/// Immutable, precomputed camera transform used during one render pass.
pub struct Projector {
    sin_yaw: f32,
    cos_yaw: f32,
    sin_pitch: f32,
    cos_pitch: f32,
    distance: f32,
    target_x: f32,
    target_y: f32,
    target_z: f32,
    focal_length: f32,
}

impl Projector {
    /// Projects a world point, returning `ScreenPoint::INVALID` when unsafe.
    pub fn project(&self, point: Point3) -> ScreenPoint {
        match self.project_with_depth(point) {
            Some(projected) => projected.screen,
            None => ScreenPoint::INVALID,
        }
    }

    /// Projects a world point and retains depth for conservative label filtering.
    pub fn project_with_depth(&self, point: Point3) -> Option<ProjectedPoint> {
        let transformed = self.transform(point);
        let depth = self.distance - transformed.y;
        let screen = self.project_transformed(transformed);
        if screen.is_visible() {
            Some(ProjectedPoint { screen, depth })
        } else {
            None
        }
    }

    /// Clips a world-space segment against the near plane, then projects it.
    /// This prevents axes/grid lines that cross the camera from generating huge
    /// or non-finite screen coordinates.
    pub fn project_line(&self, start: Point3, end: Point3) -> Option<ProjectedLine> {
        let mut start = self.transform(start);
        let mut end = self.transform(end);
        if !start.is_finite() || !end.is_finite() {
            return None;
        }

        let start_depth = self.distance - start.y;
        let end_depth = self.distance - end.y;
        if start_depth < NEAR_DEPTH && end_depth < NEAR_DEPTH {
            return None;
        }
        if start_depth < NEAR_DEPTH {
            start = clip_to_depth(start, end, NEAR_DEPTH, self.distance);
        } else if end_depth < NEAR_DEPTH {
            end = clip_to_depth(end, start, NEAR_DEPTH, self.distance);
        }

        let start = self.project_transformed(start);
        let end = self.project_transformed(end);
        if !start.is_visible() || !end.is_visible() {
            None
        } else {
            Some(ProjectedLine { start, end })
        }
    }

    fn transform(&self, point: Point3) -> CameraPoint {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return CameraPoint::INVALID;
        }
        // Translation is applied in world space before the inverse orbit rotation.
        let x = point.x - self.target_x;
        let y = point.y - self.target_y;
        let z = point.z - self.target_z;
        let rotated_x = self.cos_yaw * x - self.sin_yaw * y;
        let yawed_y = self.sin_yaw * x + self.cos_yaw * y;
        CameraPoint {
            x: rotated_x,
            y: self.cos_pitch * yawed_y - self.sin_pitch * z,
            z: self.sin_pitch * yawed_y + self.cos_pitch * z,
        }
    }

    fn project_transformed(&self, point: CameraPoint) -> ScreenPoint {
        if !point.is_finite() {
            return ScreenPoint::INVALID;
        }
        let depth = self.distance - point.y;
        if depth <= 1.0 || !depth.is_finite() {
            return ScreenPoint::INVALID;
        }

        let scale = self.focal_length / depth;
        let screen_x = SCREEN_CENTER_X + point.x * scale;
        let screen_y = SCREEN_CENTER_Y - point.z * scale;

        // Keep later i16 conversion and Bresenham traversal safely bounded even
        // for geometry just beyond the physical viewport.
        if !screen_x.is_finite()
            || !screen_y.is_finite()
            || screen_x < -512.0
            || screen_x > 831.0
            || screen_y < -512.0
            || screen_y > 751.0
        {
            return ScreenPoint::INVALID;
        }

        ScreenPoint {
            x: screen_x as i16,
            y: screen_y as i16,
        }
    }
}

#[derive(Clone, Copy)]
struct CameraPoint {
    x: f32,
    y: f32,
    z: f32,
}

impl CameraPoint {
    const INVALID: CameraPoint = CameraPoint {
        x: f32::NAN,
        y: f32::NAN,
        z: f32::NAN,
    };

    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

fn clip_to_depth(near: CameraPoint, far: CameraPoint, depth: f32, distance: f32) -> CameraPoint {
    let near_depth = distance - near.y;
    let far_depth = distance - far.y;
    let denominator = far_depth - near_depth;
    if denominator.abs() < 0.00001 {
        return near;
    }
    let t = (depth - near_depth) / denominator;
    CameraPoint {
        x: near.x + (far.x - near.x) * t,
        y: near.y + (far.y - near.y) * t,
        z: near.z + (far.z - near.z) * t,
    }
}

fn clamp(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_rejects_non_finite_points() {
        let projector = Camera::new().projector();
        assert!(!projector
            .project(Point3 {
                x: f32::NAN,
                y: 0.0,
                z: 0.0
            })
            .is_visible());
    }

    #[test]
    fn line_projection_clips_the_near_plane() {
        let camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: 8.0,
            target_x: 0.0,
            target_y: 0.0,
            target_z: 0.0,
            focal_length: 235.0,
        };
        let line = camera.projector().project_line(
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 0.0,
                y: 9.0,
                z: 0.0,
            },
        );
        assert!(line.is_some());
    }

    #[test]
    fn target_projects_to_screen_center() {
        let mut camera = Camera::new();
        camera.truck(1.0);
        camera.pedestal(0.5);
        let projected = camera.projector().project(Point3 {
            x: camera.target_x,
            y: camera.target_y,
            z: camera.target_z,
        });
        assert_eq!(projected, ScreenPoint { x: 160, y: 120 });
    }

    #[test]
    fn truck_follows_camera_horizontal_orientation() {
        let mut camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: 8.0,
            target_x: 0.0,
            target_y: 0.0,
            target_z: 0.0,
            focal_length: 235.0,
        };
        camera.truck(1.0);
        assert!((camera.target_x - 1.0).abs() < 0.0001);
        assert!(camera.target_y.abs() < 0.0001);
        camera.orbit(core::f32::consts::FRAC_PI_2, 0.0);
        camera.truck(1.0);
        // The embedded sine/cosine approximation trades a little precision for
        // code size, so verify direction rather than desktop-libm exactness.
        assert!((camera.target_x - 1.0).abs() < 0.02);
        assert!((camera.target_y + 1.0).abs() < 0.02);
    }

    #[test]
    fn dolly_changes_depth_not_focal_length_and_clamps() {
        let mut camera = Camera::new();
        let focal = camera.focal_length;
        camera.dolly(-1.0);
        assert_eq!(camera.distance, 7.0);
        assert_eq!(camera.focal_length, focal);
        camera.dolly(-100.0);
        assert_eq!(camera.distance, MIN_DISTANCE);
        camera.dolly(100.0);
        assert_eq!(camera.distance, MAX_DISTANCE);
    }

    #[test]
    fn camera_mutators_ignore_non_finite_input() {
        let mut camera = Camera::new();
        camera.orbit(f32::NAN, f32::INFINITY);
        camera.truck(f32::NAN);
        camera.pedestal(f32::INFINITY);
        camera.dolly(f32::NAN);
        camera.adjust_focal_length(f32::INFINITY);
        assert!(camera.yaw.is_finite());
        assert!(camera.pitch.is_finite());
        assert!(camera.distance.is_finite());
        assert!(camera.target_x.is_finite());
        assert!(camera.target_y.is_finite());
        assert!(camera.target_z.is_finite());
        assert!(camera.focal_length.is_finite());
    }
}
