/// Piecewise-linear interpolation over `[x, y]` points, clamped to the first and
/// last point outside their range.
///
/// Shared by the fan curve (temperature to duty) and the stall monitor's
/// expected-RPM table (duty to RPM), so the clamping and segment-selection rules
/// are defined once rather than re-derived per caller.
pub fn interpolate(points: &[[i64; 2]], x: f64) -> f64 {
    let &[first_x, first_y] = points.first().expect("points validated non-empty");
    let &[last_x, last_y] = points.last().expect("points validated non-empty");

    if x <= first_x as f64 {
        return first_y as f64;
    }
    if x >= last_x as f64 {
        return last_y as f64;
    }

    for pair in points.windows(2) {
        let (&[x0, y0], &[x1, y1]) = (&pair[0], &pair[1]);
        let (x0, y0, x1, y1) = (x0 as f64, y0 as f64, x1 as f64, y1 as f64);
        if x <= x1 {
            let fraction = (x - x0) / (x1 - x0);
            return y0 + fraction * (y1 - y0);
        }
    }

    // Only NaN reaches here: every comparison against it is false, so neither
    // clamp nor any segment matches. Do not turn this into `unreachable!()`. That
    // panic would take the whole daemon down and leave the fan wherever the
    // firmware failsafe put it. `infra::hwmon` rejects non-finite readings, so
    // nothing should produce a NaN, but a safety-critical loop should not stake
    // its life on a guard in another layer. Falling back to the last point gives
    // maximum cooling for any sane curve.
    last_y as f64
}

/// Piecewise-linear fan curve over [temp_celsius, pwm_percent] points.
/// Below the first point returns the first PWM, above the last returns the
/// last PWM.
pub fn evaluate(curve: &[[i64; 2]], temp_celsius: f64) -> u8 {
    // The cast is safe because every duty is validated into 0..=100, and
    // interpolation never leaves the range spanned by its inputs.
    interpolate(curve, temp_celsius).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVE: &[[i64; 2]] = &[[30, 20], [35, 30], [40, 50], [45, 80], [50, 100]];

    #[test]
    fn below_first_point_clamps() {
        assert_eq!(evaluate(CURVE, 10.0), 20);
        assert_eq!(evaluate(CURVE, 30.0), 20);
    }

    #[test]
    fn above_last_point_clamps() {
        assert_eq!(evaluate(CURVE, 50.0), 100);
        assert_eq!(evaluate(CURVE, 75.0), 100);
    }

    #[test]
    fn interpolates_between_points() {
        assert_eq!(evaluate(CURVE, 32.5), 25);
        assert_eq!(evaluate(CURVE, 37.5), 40);
        assert_eq!(evaluate(CURVE, 42.5), 65);
    }

    #[test]
    fn exact_points_returned() {
        assert_eq!(evaluate(CURVE, 35.0), 30);
        assert_eq!(evaluate(CURVE, 45.0), 80);
    }

    #[test]
    fn a_nan_temperature_yields_maximum_cooling_rather_than_a_panic() {
        // `unreachable!()` here would be reachable with NaN and would abort the
        // daemon. `infra::hwmon` rejects non-finite readings, so this is a
        // backstop, but it has to fail towards the fan rather than towards a crash.
        assert_eq!(evaluate(CURVE, f64::NAN), 100);
    }

    #[test]
    fn interpolate_and_evaluate_agree() {
        for temp in [10.0, 30.0, 32.5, 37.5, 42.5, 45.0, 50.0, 75.0] {
            assert_eq!(
                evaluate(CURVE, temp),
                interpolate(CURVE, temp).round() as u8,
                "at {temp} C"
            );
        }
    }

    #[test]
    fn interpolate_handles_a_duty_to_rpm_table() {
        // The stall monitor's shape: duty -> measured RPM, with a floor below the
        // knee. These are this fan's measured points.
        const EXPECTED: &[[i64; 2]] = &[[0, 840], [31, 840], [32, 900], [100, 1973]];
        assert_eq!(interpolate(EXPECTED, 0.0), 840.0);
        assert_eq!(interpolate(EXPECTED, 31.0), 840.0);
        assert_eq!(interpolate(EXPECTED, 32.0), 900.0);
        assert_eq!(interpolate(EXPECTED, 100.0), 1973.0);
        // Interpolated mid-segment, and well above the floor.
        let mid = interpolate(EXPECTED, 66.0);
        assert!(mid > 1400.0 && mid < 1450.0, "got {mid}");
    }

    #[test]
    fn single_point_curve_is_constant() {
        let flat = &[[40, 60]];
        assert_eq!(evaluate(flat, 0.0), 60);
        assert_eq!(evaluate(flat, 40.0), 60);
        assert_eq!(evaluate(flat, 90.0), 60);
    }
}
