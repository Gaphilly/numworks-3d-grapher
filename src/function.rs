use crate::math;

pub trait SurfaceFunction {
    fn evaluate(&self, x: f32, y: f32) -> f32;
}

pub struct SinCosSurface;

impl SurfaceFunction for SinCosSurface {
    fn evaluate(&self, x: f32, y: f32) -> f32 {
        let (sin_x, _) = math::sin_cos(x);
        let (_, cos_y) = math::sin_cos(y);
        sin_x * cos_y
    }
}
