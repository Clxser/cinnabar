//! FFmpeg progress is bounded and advances only when actual encoded work advances.

use std::{
    io::Read,
    process::ChildStdout,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const INACTIVITY: Duration = Duration::from_secs(300);
// FFmpeg's synchronous +faststart copy has no periodic progress callback. Reserve both read and
// write traffic at a conservative storage rate, then keep cancellation active during that allowance.
const RELOCATION_BYTES_PER_SECOND: u64 = 8 * 1024 * 1024;
const MAX_LINE: usize = 128;

pub(super) struct Finalization {
    activity: u64,
    last_progress: Instant,
    allowance: Duration,
}

impl Finalization {
    /// Inactivity, rather than total encoding/muxing duration, bounds a finishing process.
    pub(super) fn new(now: Instant, activity: u64, relocation_bytes: u64) -> Self {
        let copy_seconds = relocation_bytes
            .saturating_mul(2)
            .div_ceil(RELOCATION_BYTES_PER_SECOND);
        Self {
            activity,
            last_progress: now,
            allowance: INACTIVITY.saturating_add(Duration::from_secs(copy_seconds)),
        }
    }

    /// The caller supplies time so sustained progress and stalls can be checked without sleeping.
    pub(super) fn expired(&mut self, now: Instant, activity: u64) -> bool {
        if activity != self.activity {
            self.activity = activity;
            self.last_progress = now;
        }
        now.saturating_duration_since(self.last_progress) >= self.allowance
    }
}

/// This reader owns only FFmpeg's private progress pipe, never media pixels or unbounded lines.
pub(super) fn spawn(
    stdout: ChildStdout,
    activity: Arc<AtomicU64>,
) -> Result<JoinHandle<()>, String> {
    std::thread::Builder::new()
        .name("studio-media-progress".into())
        .spawn(move || read(stdout, &activity))
        .map_err(|error| error.to_string())
}

#[derive(Default)]
struct Values {
    frame: u64,
    time: u64,
    bytes: u64,
}

impl Values {
    /// Repeated reports, unknown fields and malformed values are not evidence of forward progress.
    fn advances(&mut self, line: &[u8]) -> bool {
        let Some((key, value)) = std::str::from_utf8(line)
            .ok()
            .and_then(|line| line.trim_end().split_once('='))
        else {
            return false;
        };
        let Ok(value) = value.trim().parse::<u64>() else {
            return false;
        };
        let previous = match key {
            "frame" => &mut self.frame,
            "out_time_us" => &mut self.time,
            "total_size" => &mut self.bytes,
            _ => return false,
        };
        if value <= *previous {
            return false;
        }
        *previous = value;
        true
    }
}

/// Overlong lines are skipped in place; the reader retains a fixed byte budget until EOF or kill.
fn read(mut input: impl Read, activity: &AtomicU64) {
    let mut chunk = [0; 512];
    let mut line = [0; MAX_LINE];
    let mut length = 0;
    let mut overlong = false;
    let mut values = Values::default();
    loop {
        let count = match input.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for &byte in &chunk[..count] {
            if byte == b'\n' {
                if !overlong && values.advances(&line[..length]) {
                    activity.fetch_add(1, Ordering::Release);
                }
                length = 0;
                overlong = false;
            } else if length < line.len() {
                line[length] = byte;
                length += 1;
            } else {
                overlong = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalization_can_progress_for_hours_but_unchanged_reports_do_not_hide_a_stall() {
        let started = Instant::now();
        let mut finish = Finalization::new(started, 0, 0);
        for activity in 1..=120 {
            assert!(!finish.expired(started + Duration::from_secs(activity * 60), activity));
        }
        let last = started + Duration::from_secs(120 * 60);
        assert!(!finish.expired(last + INACTIVITY - Duration::from_secs(1), 120));
        assert!(finish.expired(last + INACTIVITY, 120));
    }

    #[test]
    fn synchronous_large_index_relocation_has_a_byte_based_allowance_and_still_expires() {
        let started = Instant::now();
        let bytes = 64 * 1024 * 1024 * 1024;
        let mut finish = Finalization::new(started, 0, bytes);
        assert!(!finish.expired(started + Duration::from_secs(600), 0));
        let end = started
            + INACTIVITY
            + Duration::from_secs((bytes * 2).div_ceil(RELOCATION_BYTES_PER_SECOND));
        assert!(finish.expired(end, 0));
    }

    #[test]
    fn bounded_progress_reader_counts_only_advancing_numeric_work() {
        let activity = AtomicU64::new(0);
        let mut input = b"frame=1\nout_time_us=1000\ntotal_size=100\nprogress=continue\n".to_vec();
        input.extend_from_slice(b"frame=1\nout_time_us=0\ntotal_size=N/A\nframe=-1\n");
        input.extend_from_slice(&[b'x'; MAX_LINE + 1]);
        input.extend_from_slice(b"\nframe=2\nframe=3\r\n");
        read(input.as_slice(), &activity);
        assert_eq!(activity.load(Ordering::Acquire), 5);
        assert!(!Values::default().advances(b"not a progress field"));
    }
}
