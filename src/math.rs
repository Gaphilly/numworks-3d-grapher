//! Small `f32` math routines for the `no_std` embedded target.
//!
//! These approximations avoid pulling desktop math crates/runtime assumptions
//! into the relocatable NWA. They favor bounded code size and adequate graphing
//! accuracy over correctly rounded libm behavior. Domain errors return NaN;
//! overflow can return infinity, and the expression/projector layers reject all
//! non-finite results before rasterization.

const PI: f32 = 3.1415927;
const HALF_PI: f32 = 1.5707964;
const TWO_PI: f32 = 6.2831855;
/// Euler's number, exposed to the expression compiler as the `e` constant.
pub const E: f32 = 2.7182817;

/// Reduces a finite angle to approximately `[-pi, pi]` in bounded time.
///
/// Repeatedly adding or subtracting one period is unacceptable here: a valid
/// expression such as `sin(1e38)` would require an effectively unbounded
/// number of iterations on the calculator.  This reducer instead greedily
/// subtracts binary-scaled copies of `2*pi`.  A finite `f32` has at most 127
/// positive exponent steps, so both loops have a small fixed upper bound.
///
/// The usual `f32` precision limits still apply to enormous arguments.  Above
/// the range where a float can retain sub-period detail, this is a stable,
/// finite phase approximation rather than a correctly-rounded transcendental
/// reduction.  Ordinary graphing angles retain the previous subtraction-based
/// behavior to within normal `f32` rounding.
fn wrap_angle(angle: f32) -> f32 {
    if !angle.is_finite() {
        return f32::NAN;
    }

    let negative = angle < 0.0;
    let mut remainder = angle.abs();
    if remainder <= PI {
        return angle;
    }

    // `2^127` is the largest useful finite scale for a normalized f32.  The
    // half-magnitude condition prevents the next doubling from overflowing.
    const MAX_BINARY_PERIOD_STEPS: u8 = 127;
    let mut period = TWO_PI;
    let mut steps = 0_u8;
    while steps < MAX_BINARY_PERIOD_STEPS && period <= remainder * 0.5 {
        period *= 2.0;
        steps += 1;
    }

    // Process each binary multiple once.  Unlike the old loop, this is bounded
    // even for `f32::MAX`; small periods that no longer affect a huge rounded
    // remainder are simply harmless no-ops.
    loop {
        if remainder >= period {
            remainder -= period;
        }
        if steps == 0 {
            break;
        }
        period *= 0.5;
        steps -= 1;
    }

    if remainder > PI {
        remainder -= TWO_PI;
    }
    if negative {
        -remainder
    } else {
        remainder
    }
}

/// Returns sine and cosine together after range reduction, sharing polynomial work.
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

/// Tangent derived from `sin_cos`; near singularities return NaN.
pub fn tan(angle: f32) -> f32 {
    let (sin, cos) = sin_cos(angle);
    if cos.abs() < 0.0001 {
        f32::NAN
    } else {
        sin / cos
    }
}

/// Ten-iteration Newton square root; negative inputs return NaN.
pub fn sqrt(value: f32) -> f32 {
    if value < 0.0 {
        return f32::NAN;
    }
    if value == 0.0 {
        return 0.0;
    }
    let mut estimate = if value >= 1.0 { value } else { 1.0 };
    let mut iteration = 0;
    while iteration < 10 {
        estimate = 0.5 * (estimate + value / estimate);
        iteration += 1;
    }
    estimate
}

/// Greatest integral `f32` not greater than `value`.
///
/// Finite `f32` values with magnitude at least `2^23` already have no
/// fractional bits. Smaller values can be safely converted through `i32`, so
/// this avoids a platform libm dependency while preserving floor semantics.
pub fn floor(value: f32) -> f32 {
    if !value.is_finite() {
        return f32::NAN;
    }
    if value.abs() >= 8_388_608.0 {
        return value;
    }
    let truncated = (value as i32) as f32;
    if value < truncated {
        truncated - 1.0
    } else {
        truncated
    }
}

/// Smallest integral `f32` not less than `value`.
pub fn ceil(value: f32) -> f32 {
    if !value.is_finite() {
        return f32::NAN;
    }
    if value.abs() >= 8_388_608.0 {
        return value;
    }
    let truncated = (value as i32) as f32;
    if value > truncated {
        truncated + 1.0
    } else {
        truncated
    }
}

/// Rounds to the nearest integral value, with half values away from zero.
pub fn round(value: f32) -> f32 {
    if !value.is_finite() || value.abs() >= 8_388_608.0 {
        return value;
    }
    if value >= 0.0 {
        floor(value + 0.5)
    } else {
        ceil(value - 0.5)
    }
}

/// Finite numerical minimum. Any non-finite operand deliberately invalidates
/// the complete expression rather than selecting the other operand.
pub fn min(left: f32, right: f32) -> f32 {
    if !left.is_finite() || !right.is_finite() {
        f32::NAN
    } else if left <= right {
        left
    } else {
        right
    }
}

/// Finite numerical maximum. Any non-finite operand deliberately invalidates
/// the complete expression rather than selecting the other operand.
pub fn max(left: f32, right: f32) -> f32 {
    if !left.is_finite() || !right.is_finite() {
        f32::NAN
    } else if left >= right {
        left
    } else {
        right
    }
}

/// Real `f32` power. Integer exponents support negative bases; fractional
/// exponents use the local `exp(exponent * ln(base))` approximation.
pub fn pow(base: f32, exponent: f32) -> f32 {
    if exponent == 0.0 {
        return 1.0;
    }

    let integer = exponent as i32;
    if integer as f32 == exponent {
        if integer == i32::MIN {
            return f32::NAN;
        }
        let magnitude = if integer < 0 { -integer } else { integer } as u32;
        let result = integer_power(base, magnitude);
        return if integer < 0 { 1.0 / result } else { result };
    }

    if base <= 0.0 {
        return f32::NAN;
    }
    exp(exponent * ln(base))
}

fn integer_power(mut base: f32, mut exponent: u32) -> f32 {
    let mut result = 1.0_f32;
    while exponent > 0 {
        if exponent & 1 != 0 {
            result *= base;
        }
        base *= base;
        exponent >>= 1;
    }
    result
}

/// Natural logarithm approximation. Non-positive or non-finite inputs return
/// NaN so the evaluator can reject them uniformly.
pub fn ln(mut value: f32) -> f32 {
    const LN_2: f32 = 0.6931472;
    if value <= 0.0 || !value.is_finite() {
        return f32::NAN;
    }

    let mut exponent = 0_i32;
    while value >= 2.0 {
        value *= 0.5;
        exponent += 1;
    }
    while value < 1.0 {
        value *= 2.0;
        exponent -= 1;
    }

    let term = (value - 1.0) / (value + 1.0);
    let term_squared = term * term;
    let mut sum = term;
    let mut power = term;
    let mut denominator = 3.0_f32;
    let mut iteration = 0;
    while iteration < 6 {
        power *= term_squared;
        sum += power / denominator;
        denominator += 2.0;
        iteration += 1;
    }
    2.0 * sum + exponent as f32 * LN_2
}

/// Exponential approximation. Overflow is represented as infinity and is
/// rejected by the expression evaluator before it reaches graphing code.
pub fn exp(value: f32) -> f32 {
    const LN_2: f32 = 0.6931472;
    if value > 88.0 {
        return f32::INFINITY;
    }
    if value < -88.0 {
        return 0.0;
    }

    let exponent = (value / LN_2) as i32;
    let remainder = value - exponent as f32 * LN_2;
    let mut term = 1.0_f32;
    let mut sum = 1.0_f32;
    let mut divisor = 1.0_f32;
    let mut iteration = 0;
    while iteration < 9 {
        term *= remainder;
        sum += term / divisor;
        iteration += 1;
        divisor *= (iteration + 1) as f32;
    }

    if exponent >= 0 {
        sum * integer_power(2.0, exponent as u32)
    } else {
        sum / integer_power(2.0, (-exponent) as u32)
    }
}

/// Explicit-base logarithm with the mathematical domain `base > 0`,
/// `base != 1`, and `value > 0`.
pub fn log(base: f32, value: f32) -> f32 {
    if !base.is_finite() || !value.is_finite() || base <= 0.0 || base == 1.0 || value <= 0.0 {
        return f32::NAN;
    }
    ln(value) / ln(base)
}

/// Arctangent approximation with reciprocal range reduction.
///
/// The polynomial is evaluated only on `[0, 1]`, where it remains well within
/// the expression engine's graphing tolerance. Large finite arguments are
/// reduced by one reciprocal, not an unbounded loop.
pub fn atan(value: f32) -> f32 {
    if !value.is_finite() {
        return f32::NAN;
    }
    let negative = value < 0.0;
    let magnitude = value.abs();
    let reciprocal = magnitude > 1.0;
    let reduced = if reciprocal {
        1.0 / magnitude
    } else {
        magnitude
    };
    let squared = reduced * reduced;
    let polynomial = reduced
        * (0.999866
            + squared
                * (-0.3302995 + squared * (0.180141 + squared * (-0.085133 + squared * 0.020835))));
    let angle = if reciprocal {
        HALF_PI - polynomial
    } else {
        polynomial
    };
    if negative {
        -angle
    } else {
        angle
    }
}

/// Arcsine derived from the local square root and arctangent approximations.
pub fn asin(value: f32) -> f32 {
    if !value.is_finite() || value < -1.0 || value > 1.0 {
        return f32::NAN;
    }
    if value == 1.0 {
        return HALF_PI;
    }
    if value == -1.0 {
        return -HALF_PI;
    }
    atan(value / sqrt((1.0 - value) * (1.0 + value)))
}

/// Arccosine derived from the local arcsine approximation.
pub fn acos(value: f32) -> f32 {
    if !value.is_finite() || value < -1.0 || value > 1.0 {
        return f32::NAN;
    }
    HALF_PI - asin(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{} != {} within {}",
            actual,
            expected,
            tolerance
        );
    }

    #[test]
    fn integral_and_extrema_functions_follow_finite_math_semantics() {
        assert_eq!(floor(3.7), 3.0);
        assert_eq!(floor(-3.7), -4.0);
        assert_eq!(ceil(3.2), 4.0);
        assert_eq!(ceil(-3.2), -3.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -3.0);
        assert_eq!(min(2.0, 5.0), 2.0);
        assert_eq!(min(-2.0, -5.0), -5.0);
        assert_eq!(min(4.0, 4.0), 4.0);
        assert_eq!(max(2.0, 5.0), 5.0);
        assert_eq!(max(-2.0, -5.0), -2.0);
        assert_eq!(max(4.0, 4.0), 4.0);
        assert!(min(1.0, f32::NAN).is_nan());
        assert!(max(f32::INFINITY, 1.0).is_nan());
    }

    #[test]
    fn exponential_and_logarithmic_domains_are_bounded() {
        close(exp(0.0), 1.0, 0.001);
        close(ln(1.0), 0.0, 0.001);
        close(log(10.0, 100.0), 2.0, 0.002);
        close(log(2.0, 8.0), 3.0, 0.002);
        close(log(E, E), 1.0, 0.002);
        assert!(ln(0.0).is_nan());
        assert!(ln(-1.0).is_nan());
        assert!(log(1.0, 10.0).is_nan());
        assert!(log(10.0, 0.0).is_nan());
        assert!(log(-2.0, 10.0).is_nan());
    }

    #[test]
    fn inverse_trigonometric_approximations_match_host_references() {
        let inverse_sine_points = [-1.0, -0.9999, -0.9, -0.5, 0.0, 0.5, 0.9, 0.9999, 1.0];
        let inverse_tangent_points = [-100.0, -10.0, -1.0, -0.5, 0.0, 0.5, 1.0, 10.0, 100.0];
        let mut asin_error = 0.0_f32;
        let mut acos_error = 0.0_f32;
        let mut atan_error = 0.0_f32;

        for point in inverse_sine_points {
            asin_error = asin_error.max((asin(point) - point.asin()).abs());
            acos_error = acos_error.max((acos(point) - point.acos()).abs());
        }
        for point in inverse_tangent_points {
            atan_error = atan_error.max((atan(point) - point.atan()).abs());
        }

        println!(
            "inverse-trig max absolute error: asin={}, acos={}, atan={}",
            asin_error, acos_error, atan_error
        );
        assert!(asin_error <= 0.005, "asin maximum error: {}", asin_error);
        assert!(acos_error <= 0.005, "acos maximum error: {}", acos_error);
        assert!(atan_error <= 0.005, "atan maximum error: {}", atan_error);
    }

    #[test]
    fn inverse_trigonometric_domains_return_nan() {
        for value in [-1.1, 1.1] {
            assert!(asin(value).is_nan());
            assert!(acos(value).is_nan());
        }
    }

    #[test]
    fn bounded_angle_reduction_handles_large_finite_inputs() {
        for angle in [1.0e10_f32, 1.0e20_f32, 1.0e38_f32] {
            for signed_angle in [angle, -angle] {
                let reduced = wrap_angle(signed_angle);
                assert!(reduced.is_finite());
                assert!(reduced >= -PI);
                assert!(reduced <= PI);

                let (sin, cos) = sin_cos(signed_angle);
                assert!(sin.is_finite());
                assert!(cos.is_finite());

                let tangent = tan(signed_angle);
                assert!(tangent.is_finite() || tangent.is_nan());
            }
        }
    }

    #[test]
    fn ordinary_angles_keep_the_expected_quadrants() {
        let (sin, cos) = sin_cos(10.0);
        assert!((sin + 0.5440).abs() < 0.002);
        assert!((cos + 0.8391).abs() < 0.002);

        let (sin, cos) = sin_cos(-10.0);
        assert!((sin - 0.5440).abs() < 0.002);
        assert!((cos + 0.8391).abs() < 0.002);
    }
}
