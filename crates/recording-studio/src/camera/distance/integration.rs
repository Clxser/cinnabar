//! Fixed-size arc cells bracket a bounded Newton solve without speed jumps at lookup boundaries.

use super::*;

const SOLVE_STEPS: usize = 8;
const RULE: [(f64, f64); 5] = [
    (0.0, 0.5688888888888889),
    (-0.5384693101056831, 0.4786286704993665),
    (0.5384693101056831, 0.4786286704993665),
    (-0.906179845938664, 0.2369268850561891),
    (0.906179845938664, 0.2369268850561891),
];

#[cfg(test)]
thread_local! { static MEASUREMENTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// A thread-local work witness isolates quadrature reuse assertions from parallel tests.
#[cfg(test)]
pub(super) fn measurements() -> usize {
    MEASUREMENTS.with(std::cell::Cell::get)
}

/// Five-point Gauss integration is paired with a three-point estimate on the same cell.
pub(super) fn measure(points: [DVec3; 4], low: f64, high: f64) -> (f64, f64) {
    #[cfg(test)]
    MEASUREMENTS.with(|count| count.set(count.get() + 1));
    let fine = length(points, low, high);
    let center = (low + high) * 0.5;
    let half = (high - low) * 0.5;
    let offset = (3.0_f64 / 5.0).sqrt() * half;
    let coarse = scaled_speed(points, center, half * (8.0 / 9.0))
        + scaled_speed(points, center - offset, half * (5.0 / 9.0))
        + scaled_speed(points, center + offset, half * (5.0 / 9.0));
    (fine, (fine - coarse).abs())
}

/// The integral uses only its frozen control polygon and never an elapsed simulation step.
fn length(points: [DVec3; 4], low: f64, high: f64) -> f64 {
    let half = (high - low) * 0.5;
    let center = (low + high) * 0.5;
    RULE.into_iter()
        .map(|(node, weight)| scaled_speed(points, center + half * node, half * weight))
        .sum::<f64>()
}

/// Analytic cubic speed supplies the Newton derivative and both independent integration rules.
fn speed(points: [DVec3; 4], at: f64) -> f64 {
    norm(derivative(points, at))
}

/// Hypot avoids overflow from squaring otherwise representable world-space distances.
pub(super) fn norm(vector: DVec3) -> f64 {
    vector.x.hypot(vector.y).hypot(vector.z)
}

/// Quadrature weights are applied before derivatives so finite travel need not form an oversized speed.
fn scaled_speed(points: [DVec3; 4], at: f64, scale: f64) -> f64 {
    let before = 1.0 - at;
    norm(
        (points[1] - points[0]) * (scale * (3.0 * before * before))
            + (points[2] - points[1]) * (scale * (6.0 * before * at))
            + (points[3] - points[2]) * (scale * (3.0 * at * at)),
    )
}

/// Scaling before normalization preserves directions of both extremely small and large finite vectors.
pub(super) fn normalized(vector: DVec3) -> Option<DVec3> {
    let scale = vector.abs().max_element();
    (scale.is_finite() && scale > 0.0).then(|| {
        let scaled = vector / scale;
        scaled / norm(scaled)
    })
}

/// Analytic spatial derivatives also expose physical camera velocity to motion inspectors.
pub(super) fn derivative(points: [DVec3; 4], at: f64) -> DVec3 {
    let before = 1.0 - at;
    (points[1] - points[0]) * (3.0 * before * before)
        + (points[2] - points[1]) * (6.0 * before * at)
        + (points[3] - points[2]) * (3.0 * at * at)
}

/// A bracketed solve refines the lookup estimate; cusps fall back to bisection instead of dividing by zero.
pub(super) fn invert(
    points: [DVec3; 4],
    start: f64,
    end: f64,
    distance: f64,
    cell_length: f64,
    fraction: f64,
) -> f64 {
    if distance <= 0.0 {
        return start;
    }
    if distance >= cell_length {
        return end;
    }
    let mut low = start;
    let mut high = end;
    let mut at = start + (end - start) * fraction;
    for _ in 0..SOLVE_STEPS {
        let error = length(points, start, at) - distance;
        if error.abs() <= cell_length * 1e-11 + 1e-12 {
            break;
        }
        if error < 0.0 {
            low = at;
        } else {
            high = at;
        }
        let derivative = speed(points, at);
        let next = at - error / derivative;
        at = if derivative > 0.0 && next > low && next < high {
            next
        } else {
            (low + high) * 0.5
        };
    }
    at.clamp(start, end)
}
