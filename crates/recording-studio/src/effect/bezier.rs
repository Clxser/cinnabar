//! Weighted time/value handles have one deterministic inversion for sampling and exact parametric area.

const SCALE: f64 = u16::MAX as f64;

/// Returns power-basis coefficients for the cubic with fixed zero/one endpoints.
fn coefficients(a: f64, b: f64) -> [f64; 4] {
    [0.0, 3.0 * a, 3.0 * b - 6.0 * a, 1.0 + 3.0 * a - 3.0 * b]
}

/// Evaluates a cubic without allocating per graph point or output frame.
fn value(c: [f64; 4], at: f64) -> f64 {
    ((c[3] * at + c[2]) * at + c[1]) * at
}

/// Calculates the native derivative of the same polynomial used for sampling.
fn slope(c: [f64; 4], at: f64) -> f64 {
    (3.0 * c[3] * at + 2.0 * c[2]) * at + c[1]
}

/// Every admitted handle lies in the unit square; the time cubic is monotone even at flat tangents.
fn parameter(x: [f64; 4], at: f64) -> f64 {
    if at <= 0.0 {
        return 0.0;
    }
    if at >= 1.0 {
        return 1.0;
    }
    let (mut low, mut high) = (0.0, 1.0);
    let mut current = at;
    for _ in 0..10 {
        let error = value(x, current) - at;
        if error == 0.0 {
            return current;
        }
        if error < 0.0 {
            low = current;
        } else {
            high = current;
        }
        let candidate = current - error / slope(x, current);
        let next = if candidate > low && candidate < high {
            candidate
        } else {
            (low + high) * 0.5
        };
        if next == current {
            return current;
        }
        current = next;
    }
    for _ in 0..42 {
        let mid = (low + high) * 0.5;
        if value(x, mid) < at {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) * 0.5
}

/// Converts persisted integer controls once into the normalized parametric power basis.
fn curves(a: [u16; 2], b: [u16; 2]) -> [[f64; 4]; 2] {
    std::array::from_fn(|axis| coefficients(f64::from(a[axis]) / SCALE, f64::from(b[axis]) / SCALE))
}

/// Weighted interpolation retains exact segment endpoints and never overshoots admitted key values.
pub(super) fn sample(a: [u16; 2], b: [u16; 2], at: f64) -> f64 {
    if at <= 0.0 {
        return 0.0;
    }
    if at >= 1.0 {
        return 1.0;
    }
    let [x, y] = curves(a, b);
    value(y, parameter(x, at)).clamp(0.0, 1.0)
}

/// A finite slope keeps speed-map diagnostics usable at vertical authored handles.
pub(super) fn derivative(a: [u16; 2], b: [u16; 2], at: f64) -> f64 {
    let [x, y] = curves(a, b);
    let t = parameter(x, at);
    slope(y, t) / slope(x, t).max(1e-12)
}

/// Integrates y(t) dx(t) analytically so speed retiming and playback use the same authored curve.
pub(super) fn integral(a: [u16; 2], b: [u16; 2], at: f64) -> f64 {
    let [x, y] = curves(a, b);
    let t = parameter(x, at);
    let mut area = 0.0;
    for (i, cy) in y.into_iter().enumerate() {
        for (j, cx) in x.into_iter().enumerate().skip(1) {
            let power = i + j;
            area += cy * j as f64 * cx * t.powi(power as i32) / power as f64;
        }
    }
    area.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use crate::effect::Easing;

    #[test]
    fn weighted_linear_graph_has_matching_value_velocity_and_area() {
        let ease = Easing::Bezier {
            outgoing: [12_000, 12_000],
            incoming: [57_000, 57_000],
        };
        for n in 0..=100 {
            let at = f64::from(n) / 100.0;
            assert!((ease.sample(at) - at).abs() < 1e-12);
            assert!((ease.derivative(at) - 1.0).abs() < 1e-10);
            assert!((ease.integral(at) - at * at / 2.0).abs() < 1e-12);
        }
    }

    #[test]
    fn weighted_area_matches_dense_integration_and_handles_flat_time_tangents() {
        for ease in [
            Easing::Bezier {
                outgoing: [0, 45_000],
                incoming: [u16::MAX, 15_000],
            },
            Easing::Bezier {
                outgoing: [u16::MAX, 0],
                incoming: [0, u16::MAX],
            },
        ] {
            assert_eq!(ease.sample(0.0), 0.0);
            assert_eq!(ease.sample(1.0), 1.0);
            let sum: f64 = (0..20_000)
                .map(|n| ease.sample((f64::from(n) + 0.5) / 20_000.0) / 20_000.0)
                .sum();
            assert!((ease.integral(1.0) - sum).abs() < 1e-6);
            assert!(ease.derivative(0.5).is_finite());
        }
    }
}
