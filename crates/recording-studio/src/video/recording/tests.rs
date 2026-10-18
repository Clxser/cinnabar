use super::*;
use std::{cell::Cell, rc::Rc};

struct Deadline(Rc<Cell<usize>>);

impl Drop for Deadline {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

struct CheckedWriter {
    active: Rc<Cell<usize>>,
    data: Vec<u8>,
    writes: usize,
    fail: bool,
    cancel: Option<Cancellation>,
}

impl Write for CheckedWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        assert_eq!(self.active.get(), 1, "a pipe operation needs a deadline");
        self.writes += 1;
        if let Some(cancellation) = &self.cancel {
            cancellation.cancel();
        }
        if self.fail {
            return Err(io::Error::other("pipe stopped"));
        }
        let count = data.len().min(3);
        self.data.extend_from_slice(&data[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        assert_eq!(self.active.get(), 1);
        Ok(())
    }
}

/// Partial pipe writes exercise deadline renewal without a subprocess, a clock or sleeping.
fn checked_writer(active: &Rc<Cell<usize>>) -> CheckedWriter {
    CheckedWriter {
        active: Rc::clone(active),
        data: Vec::new(),
        writes: 0,
        fail: false,
        cancel: None,
    }
}

/// Guard ownership mirrors the supervisor's RAII IO deadline.
fn deadline(active: &Rc<Cell<usize>>) -> Deadline {
    assert_eq!(active.get(), 0);
    active.set(1);
    Deadline(Rc::clone(active))
}

#[test]
fn live_capture_keeps_frame_timing_and_deadlines_cover_only_actual_pipe_operations() {
    let active = Rc::new(Cell::new(0));
    let cancellation = Cancellation::default();
    let mut input = GuardedInput {
        inner: checked_writer(&active),
        cancellation: &cancellation,
        deadline: || deadline(&active),
    };
    let frames = [2, 4].map(|index| Frame {
        index,
        size: [2, 2],
        layout: PixelLayout::Bgra,
        data: vec![index as u8; 16],
    });
    let frames = frames.into_iter().inspect(|_| {
        assert_eq!(
            active.get(),
            0,
            "waiting for a frame must not arm IO expiry"
        );
    });
    let written = write_live(
        &mut input,
        frames,
        [2, 2],
        PixelLayout::Bgra,
        &AtomicU64::new(6),
        FrameRate {
            numerator: 240,
            denominator: 1,
        },
    )
    .unwrap();
    input.flush().unwrap();
    assert_eq!(written, 6);
    assert_eq!(input.inner.data, [vec![2; 64], vec![4; 32]].concat());
    assert!(input.inner.writes > written as usize);
    assert_eq!(active.get(), 0);
}

#[test]
fn pipe_errors_and_cancellation_release_the_deadline_without_another_write() {
    for fail in [false, true] {
        let active = Rc::new(Cell::new(0));
        let cancellation = Cancellation::default();
        let mut writer = checked_writer(&active);
        writer.fail = fail;
        writer.cancel = Some(cancellation.clone());
        let mut input = GuardedInput {
            inner: writer,
            cancellation: &cancellation,
            deadline: || deadline(&active),
        };
        assert!(input.write_all(&[1; 16]).is_err());
        assert_eq!(active.get(), 0);
        assert!(cancellation.cancelled());
        assert_eq!(input.inner.writes, 1);
        assert!(input.write_all(&[2; 16]).is_err());
        assert!(input.flush().is_err());
        assert_eq!(input.inner.writes, 1);
        assert_eq!(active.get(), 0);
    }
}

#[test]
fn a_pipe_error_releases_the_deadline_without_requiring_cancellation() {
    let active = Rc::new(Cell::new(0));
    let cancellation = Cancellation::default();
    let mut writer = checked_writer(&active);
    writer.fail = true;
    let mut input = GuardedInput {
        inner: writer,
        cancellation: &cancellation,
        deadline: || deadline(&active),
    };
    assert_eq!(
        input.write_all(&[1; 16]).unwrap_err().to_string(),
        "pipe stopped"
    );
    assert_eq!(active.get(), 0);
    assert!(!cancellation.cancelled());
    assert_eq!(input.inner.writes, 1);
}

#[test]
fn a_pipe_error_also_retains_the_supervisors_timeout_diagnosis() {
    let error = finish_result(
        Err(io::Error::other("broken pipe")),
        Err("inactivity limit".into()),
    )
    .unwrap_err();
    assert!(error.contains("broken pipe"));
    assert!(error.contains("inactivity limit"));
    assert_eq!(finish_result(Ok(240), Ok(())).unwrap(), 240);
    assert_eq!(
        finish_result(Ok(240), Err("stalled".into())).unwrap_err(),
        "stalled"
    );
    assert_eq!(
        finish_result(Err(io::Error::other("missing frame")), Ok(())).unwrap_err(),
        "missing frame"
    );
}

#[test]
fn failed_mux_publication_retains_the_completed_mux_and_original_take() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("blocked.mp4");
    std::fs::create_dir(&target).unwrap();
    let original = target.join("silent.mp4");
    std::fs::write(&original, b"silent take").unwrap();
    let staged = tempfile::NamedTempFile::new_in(directory.path())
        .unwrap()
        .into_temp_path();
    std::fs::write(&staged, b"completed mux").unwrap();
    let retained = staged.to_path_buf();
    let error = publish_mux(staged, &target).unwrap_err();
    assert!(error.contains(&retained.display().to_string()));
    assert_eq!(std::fs::read(retained).unwrap(), b"completed mux");
    assert_eq!(std::fs::read(original).unwrap(), b"silent take");
}

#[test]
fn successful_mux_publication_replaces_the_silent_take() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("video.mp4");
    std::fs::write(&target, b"silent take").unwrap();
    let staged = tempfile::NamedTempFile::new_in(directory.path())
        .unwrap()
        .into_temp_path();
    std::fs::write(&staged, b"completed mux").unwrap();
    let former_path = staged.to_path_buf();
    publish_mux(staged, &target).unwrap();
    assert!(!former_path.exists());
    assert_eq!(std::fs::read(target).unwrap(), b"completed mux");
}
