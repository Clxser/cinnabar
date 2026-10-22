use super::*;
use crate::{
    FrameRate,
    model::{Encoder, ExportCodec},
    video::{EncodeOptions, EncodingSpeed, PixelLayout, VideoSettings},
};
use std::io::{self, Cursor};

#[test]
fn binary_output_cannot_grow_past_shared_native_or_audio_admission() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("video.mp4");
    let budget = DiskBudget::new(10);
    budget.adopt(7).unwrap();
    let mut file = Metered::file(std::fs::File::create(&path).unwrap(), budget.clone()).unwrap();
    let error = copy(
        &mut Cursor::new([2; 4]),
        &mut file,
        &Cancellation::default(),
        || {},
    )
    .unwrap_err();
    assert!(error.contains("Take-wide"));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    assert_eq!(budget.used(), 7);
    copy(
        &mut Cursor::new([1; 3]),
        &mut file,
        &Cancellation::default(),
        || {},
    )
    .unwrap();
    file.sync_all().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), [1; 3]);
    assert_eq!(budget.used(), 10);
}

struct CancelOnRead {
    cancellation: Cancellation,
}
impl Read for CancelOnRead {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.cancellation.cancel();
        output[0] = 1;
        Ok(1)
    }
}

#[test]
fn cancellation_during_read_prevents_publication_and_cleanup_refunds_staging() {
    let temporary = tempfile::tempdir().unwrap();
    let budget = DiskBudget::new(10);
    let mut staged = StagedFile::new(temporary.path(), "mux-", ".part", budget.clone()).unwrap();
    staged.writer().unwrap().write_all(&[2; 3]).unwrap();
    let cancellation = Cancellation::default();
    let mut input = CancelOnRead {
        cancellation: cancellation.clone(),
    };
    assert!(
        copy(
            &mut input,
            &mut staged.writer().unwrap(),
            &cancellation,
            || panic!("cancelled output must not publish progress")
        )
        .is_err()
    );
    assert_eq!(budget.used(), 3);
    drop(staged);
    assert_eq!(budget.used(), 0);
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), 0);
}

#[test]
fn completed_binary_output_checks_cancellation_even_at_eof() {
    struct CancelAtEof(Cancellation);
    impl Read for CancelAtEof {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            self.0.cancel();
            Ok(0)
        }
    }
    let cancellation = Cancellation::default();
    assert!(
        copy(
            &mut CancelAtEof(cancellation.clone()),
            &mut Vec::new(),
            &cancellation,
            || {}
        )
        .is_err()
    );
}

#[test]
fn live_fixed_clock_and_audio_mux_all_request_seek_free_binary_mp4() {
    let settings = VideoSettings {
        path: "video.mp4".into(),
        fps: FrameRate {
            numerator: 60,
            denominator: 1,
        },
        codec: ExportCodec::H264,
    };
    for speed in [EncodingSpeed::Live, EncodingSpeed::Export] {
        let options = EncodeOptions {
            speed,
            ..Default::default()
        };
        let argv = super::super::super::options::arguments(
            &settings,
            [16, 16],
            PixelLayout::Bgra,
            &settings.path,
            options,
            Encoder::Cpu,
        );
        let argv = arguments(argv).unwrap();
        assert_eq!(&argv[argv.len() - 3..], ["-f", "mp4", PIPE]);
        let flags = argv.iter().position(|arg| arg == "-movflags").unwrap();
        assert_eq!(
            argv[flags + 1],
            super::super::super::options::STREAMING_MP4_FLAGS
        );
        assert!(!argv.iter().any(|arg| arg == "+faststart"));
    }
    let argv = arguments(super::super::mux_arguments(
        Path::new("video.mp4"),
        Path::new("audio.wav"),
        Path::new("unused.mp4"),
    ))
    .unwrap();
    assert_eq!(argv.last().unwrap(), PIPE);
    assert!(arguments(vec!["-movflags".into()]).is_err());
    assert!(arguments(Vec::new()).is_err());
}

#[test]
fn abandoned_output_task_cancels_its_owner_and_joins_the_storage_worker() {
    let cancellation = Cancellation::default();
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = finished.clone();
    let worker = std::thread::spawn(move || {
        signal.store(true, std::sync::atomic::Ordering::Release);
        Err("cancelled owned output".into())
    });
    drop(OutputTask {
        worker: Some(worker),
        cancellation: cancellation.clone(),
    });
    assert!(cancellation.cancelled());
    assert!(finished.load(std::sync::atomic::Ordering::Acquire));
}
