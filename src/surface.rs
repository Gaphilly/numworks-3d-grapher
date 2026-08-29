use crate::math;

pub const COLUMNS: usize = 25;
pub const ROWS: usize = 19;

#[derive(Clone, Copy)]
pub struct Point3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub fn point(column: usize, row: usize) -> Point3 {
    const MIN: f32 = -3.1415927;
    const SPAN: f32 = 6.2831855;

    let x = MIN + SPAN * column as f32 / (COLUMNS - 1) as f32;
    let y = MIN + SPAN * row as f32 / (ROWS - 1) as f32;
    let (sin_x, _) = math::sin_cos(x);
    let (_, cos_y) = math::sin_cos(y);
    Point3 {
        x,
        y,
        z: sin_x * cos_y,
    }
}
