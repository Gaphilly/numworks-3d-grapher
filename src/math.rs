const PI: f32 = 3.1415927;
const HALF_PI: f32 = 1.5707964;
const TWO_PI: f32 = 6.2831855;

fn wrap_angle(mut angle: f32) -> f32 {
    while angle > PI {
        angle -= TWO_PI;
    }
    while angle < -PI {
        angle += TWO_PI;
    }
    angle
}

pub fn sin_cos(angle: f32) -> (f32, f32) {
    let mut x = wrap_angle(angle);
    let mut cosine_sign = 1.0;

    if x > HALF_PI {
        x = PI - x;
        cosine_sign = -1.0;
    } else if x < -HALF_PI {
        x = -PI - x;
        cosine_sign = -1.0;
    }

    let x2 = x * x;
    let sin = x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 - x2 / 5040.0)));
    let cos = cosine_sign * (1.0 + x2 * (-1.0 / 2.0 + x2 * (1.0 / 24.0 - x2 / 720.0)));
    (sin, cos)
}
