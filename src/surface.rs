use crate::function::SurfaceFunction;

pub const COLUMNS: usize = 25;
pub const ROWS: usize = 19;

#[derive(Clone, Copy)]
pub struct Point3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub fn point<F: SurfaceFunction>(column: usize, row: usize, function: &F) -> Point3 {
    const MIN: f32 = -3.1415927;
    const SPAN: f32 = 6.2831855;

    let x = MIN + SPAN * column as f32 / (COLUMNS - 1) as f32;
    let y = MIN + SPAN * row as f32 / (ROWS - 1) as f32;
    Point3 {
        x,
        y,
        z: function.evaluate(x, y),
    }
}
