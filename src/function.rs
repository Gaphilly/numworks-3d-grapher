//! Narrow boundary between mathematical evaluation and surface sampling.

/// Evaluates a height function `z = f(x, y)` using calculator-friendly `f32`.
///
/// The renderer depends only on sampled points and never on parser/bytecode
/// details. A future expression implementation can replace the current compiler
/// without changing camera or rasterization code. Implementations should return
/// a non-finite value for undefined inputs; projection filters those samples.
pub trait SurfaceFunction {
    /// Returns the surface height at one world-domain coordinate.
    fn evaluate(&self, x: f32, y: f32) -> f32;
}
