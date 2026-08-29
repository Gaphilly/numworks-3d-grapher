use crate::math;
use crate::surface::Point3;

const SCREEN_CENTER_X: f32 = 160.0;
const SCREEN_CENTER_Y: f32 = 120.0;

#[derive(Clone, Copy)]
pub struct ScreenPoint {
    pub x: i16,
    pub y: i16,
}

impl ScreenPoint {
    pub const INVALID: ScreenPoint = ScreenPoint { x: i16::MIN, y: 0 };

    pub fn is_visible(self) -> bool {
        self.x != i16::MIN
    }
}

pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
}

impl Camera {
    pub const fn new() -> Camera {
        Camera {
            yaw: -0.65,
            pitch: 0.65,
            zoom: 1.0,
        }
    }

    pub fn projector(&self) -> Projector {
        let (sin_yaw, cos_yaw) = math::sin_cos(self.yaw);
        let (sin_pitch, cos_pitch) = math::sin_cos(self.pitch);
        Projector {
            sin_yaw,
            cos_yaw,
            sin_pitch,
            cos_pitch,
            zoom: self.zoom,
        }
    }
}

pub struct Projector {
    sin_yaw: f32,
    cos_yaw: f32,
    sin_pitch: f32,
    cos_pitch: f32,
    zoom: f32,
}

impl Projector {
    pub fn project(&self, point: Point3) -> ScreenPoint {
        if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
            return ScreenPoint::INVALID;
        }
        let rotated_x = self.cos_yaw * point.x - self.sin_yaw * point.y;
        let yawed_y = self.sin_yaw * point.x + self.cos_yaw * point.y;
        let rotated_y = self.cos_pitch * yawed_y - self.sin_pitch * point.z;
        let rotated_z = self.sin_pitch * yawed_y + self.cos_pitch * point.z;
        let depth = 8.0 - rotated_y;

        if depth <= 1.0 {
            return ScreenPoint::INVALID;
        }

        let scale = 235.0 * self.zoom / depth;
        let screen_x = SCREEN_CENTER_X + rotated_x * scale;
        let screen_y = SCREEN_CENTER_Y - rotated_z * scale;

        if screen_x < -512.0 || screen_x > 831.0 || screen_y < -512.0 || screen_y > 751.0 {
            return ScreenPoint::INVALID;
        }

        ScreenPoint {
            x: screen_x as i16,
            y: screen_y as i16,
        }
    }
}
