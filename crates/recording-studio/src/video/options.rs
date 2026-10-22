//! Encoder policy and an actual runtime hardware capability probe, confined to the encoding worker.

use super::*;
pub(super) const STREAMING_MP4_FLAGS: &str = "+frag_keyframe+empty_moov+default_base_moof";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EncodingSpeed {
    Live,
    #[default]
    Export,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeOptions {
    pub encoder: Encoder,
    /// None retains each codec's original capture quality.
    pub quality: Option<u8>,
    pub speed: EncodingSpeed,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            encoder: Encoder::Cpu,
            quality: None,
            speed: EncodingSpeed::Export,
        }
    }
}

impl EncodeOptions {
    /// Invalid controls fail before creating output files or launching a video encoder.
    pub fn validate(self) -> Result<(), String> {
        if self.quality.is_some_and(|quality| quality > 51) {
            return Err("video quality must be between zero and 51".into());
        }
        Ok(())
    }

    /// Compiled encoder names alone do not prove driver/device support; this probes one private frame.
    pub(super) fn select_encoder(
        self,
        ffmpeg: &Path,
        codec: ExportCodec,
    ) -> Result<Encoder, String> {
        if self.encoder == Encoder::Cpu {
            return Ok(Encoder::Cpu);
        }
        let mut probe = Command::new(ffmpeg);
        probe
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=s=320x240:r=1",
                "-frames:v",
                "1",
                "-an",
                "-c:v",
                encoder_name(codec, Encoder::Nvidia),
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::media_process::hide_console(&mut probe);
        let supported = probe.spawn().is_ok_and(|mut child| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => return status.success(),
                    Ok(None) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return false;
                    }
                }
            }
        });
        match (self.encoder, supported) {
            (_, true) => Ok(Encoder::Nvidia),
            (Encoder::Auto, false) => Ok(Encoder::Cpu),
            (Encoder::Nvidia, false) => Err("NVIDIA video encoding is unavailable for this format. Select automatic or CPU encoding.".into()),
            (Encoder::Cpu, false) => Ok(Encoder::Cpu),
        }
    }
}

/// Every supported codec has one software encoder and one NVIDIA encoder in this policy.
pub(super) fn encoder_name(codec: ExportCodec, encoder: Encoder) -> &'static str {
    match (codec, encoder) {
        (ExportCodec::H264, Encoder::Cpu | Encoder::Auto) => "libx264",
        (ExportCodec::Hevc, Encoder::Cpu | Encoder::Auto) => "libx265",
        (ExportCodec::Av1, Encoder::Cpu | Encoder::Auto) => "libsvtav1",
        (ExportCodec::H264, Encoder::Nvidia) => "h264_nvenc",
        (ExportCodec::Hevc, Encoder::Nvidia) => "hevc_nvenc",
        (ExportCodec::Av1, Encoder::Nvidia) => "av1_nvenc",
    }
}

/// Builds argv directly; no command line is interpreted by a shell.
pub(super) fn arguments(
    settings: &VideoSettings,
    size: [u32; 2],
    layout: PixelLayout,
    output: &Path,
    options: EncodeOptions,
    encoder: Encoder,
) -> Vec<String> {
    let [width, height] = size;
    let mut args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        layout.ffmpeg_name(),
        "-video_size",
    ]
    .map(str::to_owned)
    .into();
    args.extend([
        format!("{width}x{height}"),
        "-framerate".into(),
        settings.fps.to_string(),
        "-i".into(),
        "-".into(),
        "-an".into(),
        "-c:v".into(),
        encoder_name(settings.codec, encoder).into(),
    ]);
    if options.speed == EncodingSpeed::Live {
        let threads = live_threads_for(std::thread::available_parallelism().map_or(1, usize::from));
        args.extend(["-filter_threads".into(), "1".into()]);
        if encoder == Encoder::Cpu {
            args.extend(["-threads:v".into(), threads.to_string()]);
            match settings.codec {
                ExportCodec::H264 => {
                    args.extend(["-x264-params".into(), "lookahead-threads=1".into()])
                }
                ExportCodec::Hevc => args.extend([
                    "-x265-params".into(),
                    format!("pools={threads}:frame-threads=1"),
                ]),
                ExportCodec::Av1 => args.extend(["-svtav1-params".into(), format!("lp={threads}")]),
            }
        }
    }
    let quality = options
        .quality
        .unwrap_or(match settings.codec {
            ExportCodec::H264 => 16,
            ExportCodec::Hevc => 18,
            ExportCodec::Av1 => 20,
        })
        .to_string();
    if encoder == Encoder::Nvidia {
        args.extend(
            [
                "-preset",
                if options.speed == EncodingSpeed::Live {
                    "p3"
                } else {
                    "p5"
                },
                "-tune",
                "hq",
                "-rc",
                "vbr",
                "-cq",
            ]
            .map(str::to_owned),
        );
        args.extend([quality, "-b:v".into(), "0".into()]);
    } else {
        args.push("-preset".into());
        args.push(
            match settings.codec {
                ExportCodec::H264 | ExportCodec::Hevc => {
                    if options.speed == EncodingSpeed::Live {
                        "veryfast"
                    } else {
                        "medium"
                    }
                }
                ExportCodec::Av1 => {
                    if options.speed == EncodingSpeed::Live {
                        "10"
                    } else {
                        "6"
                    }
                }
            }
            .into(),
        );
        args.extend(["-crf".into(), quality]);
    }
    if settings.codec == ExportCodec::Hevc {
        args.extend(["-tag:v", "hvc1"].map(str::to_owned));
    }
    args.extend(
        [
            "-vf",
            "crop=trunc(iw/2)*2:trunc(ih/2)*2",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            if options.speed == EncodingSpeed::Live {
                STREAMING_MP4_FLAGS
            } else {
                "+faststart"
            },
        ]
        .map(str::to_owned),
    );
    args.push(output.display().to_string());
    args
}

/// Live software encoding shares the host with animation, streaming and rendering; export remains free.
fn live_threads_for(available: usize) -> usize {
    (available / 2).clamp(1, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_workers_leave_host_capacity_and_have_a_finite_upper_bound() {
        for (available, expected) in [(0, 1), (1, 1), (2, 1), (4, 2), (16, 8), (128, 8)] {
            assert_eq!(live_threads_for(available), expected);
        }
    }

    #[test]
    fn live_thread_limits_preserve_rate_dimensions_quality_and_encoder_choice() {
        for codec in [ExportCodec::H264, ExportCodec::Hevc, ExportCodec::Av1] {
            let settings = VideoSettings {
                path: "capture.mp4".into(),
                fps: FrameRate {
                    numerator: 240,
                    denominator: 1,
                },
                codec,
            };
            let args = arguments(
                &settings,
                [1920, 1080],
                PixelLayout::Rgba,
                &settings.path,
                EncodeOptions {
                    speed: EncodingSpeed::Live,
                    quality: Some(19),
                    encoder: Encoder::Cpu,
                },
                Encoder::Cpu,
            );
            let value = |flag: &str| {
                args.windows(2)
                    .find(|pair| pair[0] == flag)
                    .map(|pair| pair[1].as_str())
            };
            assert_eq!(value("-framerate"), Some("240"));
            assert_eq!(value("-video_size"), Some("1920x1080"));
            assert_eq!(value("-crf"), Some("19"));
            assert_eq!(value("-c:v"), Some(encoder_name(codec, Encoder::Cpu)));
            assert!(value("-threads:v").unwrap().parse::<usize>().unwrap() <= 8);
            let export = arguments(
                &settings,
                [1920, 1080],
                PixelLayout::Rgba,
                &settings.path,
                EncodeOptions::default(),
                Encoder::Cpu,
            );
            assert!(!export.iter().any(|arg| arg == "-threads:v"));
        }
    }
}
