pub trait SurfaceFunction {
    fn evaluate(&self, x: f32, y: f32) -> f32;
}
