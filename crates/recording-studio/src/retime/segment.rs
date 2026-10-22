//! Analytic inverse-speed integrals avoid frame-step accumulation and numerical quadrature.

use super::*;

#[derive(Clone, Debug)]
pub(super) struct Segment {
    pub start: f64,
    pub end: f64,
    pub output_start: f64,
    pub output_end: f64,
    first: f64,
    last: f64,
    easing: Easing,
}

impl Segment {
    /// Exact polynomial integrals preserve a ramp's endpoints independently of sampling cadence.
    pub fn output_at(&self, source: f64) -> f64 {
        let t = ((source - self.start) / (self.end - self.start)).clamp(0.0, 1.0);
        self.output_start
            + (self.end - self.start)
                * (self.first * t + (self.last - self.first) * self.easing.integral(t))
    }

    /// A bounded safeguarded Newton solve inverts a strictly positive segment without retained integration state.
    pub fn source_at(&self, output: f64) -> f64 {
        if output <= self.output_start {
            return self.start;
        }
        if output >= self.output_end {
            return self.end;
        }
        let mut low = self.start;
        let mut high = self.end;
        let mut source = low
            + (high - low) * (output - self.output_start) / (self.output_end - self.output_start);
        for _ in 0..48 {
            let error = self.output_at(source) - output;
            if error.abs() <= 1e-6 || high - low <= 1e-5 {
                break;
            }
            if error > 0.0 {
                high = source;
            } else {
                low = source;
            }
            let wanted = source - error / self.inverse_rate(source);
            let next = if wanted > low && wanted < high {
                wanted
            } else {
                (low + high) * 0.5
            };
            if next == source {
                break;
            }
            source = next;
        }
        source
    }

    /// Eased duration per source microsecond stays between the admitted positive endpoint rates.
    pub fn inverse_rate(&self, source: f64) -> f64 {
        let t = ((source - self.start) / (self.end - self.start)).clamp(0.0, 1.0);
        self.first + (self.last - self.first) * self.easing.sample(t)
    }
}

/// One constant tail on each side makes trims and transitions independent of key insertion at clip edges.
pub(super) fn compile(base: PlaybackRate, keys: &[SpeedKey]) -> Vec<Segment> {
    let inverse = |rate: PlaybackRate| {
        f64::from(base.denominator) / f64::from(base.numerator) * f64::from(rate.denominator)
            / f64::from(rate.numerator)
    };
    let mut segments = Vec::with_capacity(keys.len() + 1);
    let mut push = |start: u64, end: u64, first: f64, last: f64, easing: Easing| {
        if start == end {
            return;
        }
        let output_start = segments
            .last()
            .map_or(0.0, |segment: &Segment| segment.output_end);
        let mut segment = Segment {
            start: start as f64,
            end: end as f64,
            output_start,
            output_end: 0.0,
            first,
            last,
            easing,
        };
        segment.output_end = segment.output_at(end as f64);
        segments.push(segment);
    };
    push(
        0,
        keys[0].at.0,
        inverse(keys[0].rate),
        inverse(keys[0].rate),
        Easing::Linear,
    );
    for pair in keys.windows(2) {
        push(
            pair[0].at.0,
            pair[1].at.0,
            inverse(pair[0].rate),
            inverse(pair[1].rate),
            pair[1].easing,
        );
    }
    let last = keys.last().expect("admitted nonempty ramp");
    push(
        last.at.0,
        MAX_MEDIA_MICROS,
        inverse(last.rate),
        inverse(last.rate),
        Easing::Linear,
    );
    segments
}
