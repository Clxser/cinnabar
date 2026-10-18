//! Worker-only tool discovery and private-frame probes report actual codec/device usability.

use super::*;
use crate::{export::Cancellation, media_process::MediaChild};
use std::{io::Read, time::Duration};

const OUTPUT_BYTES: u64 = 256 * 1024;
const MAX_ENCODERS: usize = 512;
const PROBE_SIZE: [u32; 2] = [320, 240];
const TOOL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub struct EncoderProbe {
    pub encoder: String,
    pub available: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub executable: PathBuf,
    pub ffprobe: Result<PathBuf, String>,
    pub version: String,
    pub codec: ExportCodec,
    /// Compiled names are informational; only a completed frame probe proves that a driver/device works.
    pub compiled_video_encoders: Vec<String>,
    pub software: EncoderProbe,
    pub nvidia: EncoderProbe,
    pub cpu_fallback: bool,
}

/// Every check repeats discovery, so installing tools or drivers can be retried without restarting Cinnabar.
pub fn check(codec: ExportCodec, cancellation: &Cancellation) -> Result<Report, String> {
    cancellation.check()?;
    let executable = find_ffmpeg()?;
    let version = text(&executable, "-version", cancellation)?
        .lines()
        .next()
        .unwrap_or("FFmpeg version unavailable")
        .chars()
        .take(512)
        .collect();
    let compiled_video_encoders = parse_encoders(&text(&executable, "-encoders", cancellation)?)?;
    let ffprobe = crate::media::probe_executable(&executable).and_then(|path| {
        text(&path, "-version", cancellation)
            .map(|_| path)
            .map_err(|error| {
                format!("FFprobe could not run: {error}. Reinstall FFprobe beside FFmpeg.")
            })
    });
    let software = probe(
        &executable,
        codec,
        Encoder::Cpu,
        &compiled_video_encoders,
        cancellation,
    );
    cancellation.check()?;
    let nvidia = probe(
        &executable,
        codec,
        Encoder::Nvidia,
        &compiled_video_encoders,
        cancellation,
    );
    cancellation.check()?;
    Ok(Report {
        executable,
        ffprobe,
        version,
        codec,
        cpu_fallback: software.available && !nvidia.available,
        compiled_video_encoders,
        software,
        nvidia,
    })
}

/// Output is byte bounded and the supervised process has a deadline even while its pipe is blocked.
fn text(path: &Path, argument: &str, cancellation: &Cancellation) -> Result<String, String> {
    let mut command = Command::new(path);
    command
        .args(["-hide_banner", argument])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = MediaChild::start_timed(command, cancellation.clone(), TOOL_TIMEOUT)?;
    let mut output = Vec::new();
    child
        .stdout
        .as_mut()
        .ok_or("The media tool did not expose diagnostic output.")?
        .take(OUTPUT_BYTES + 1)
        .read_to_end(&mut output)
        .map_err(|error| error.to_string())?;
    if output.len() as u64 > OUTPUT_BYTES {
        return Err("Media tool diagnostics exceeded their output byte limit.".into());
    }
    child.finish()?;
    String::from_utf8(output)
        .map_err(|_| "The media tool returned invalid UTF-8 diagnostics.".into())
}

/// FFmpeg's machine-readable encoder flags identify video entries without assuming any installed GPU vendor.
fn parse_encoders(output: &str) -> Result<Vec<String>, String> {
    let mut found = std::collections::BTreeSet::new();
    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let (Some(flags), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        if flags.len() != 6 || !flags.starts_with('V') || name == "=" {
            continue;
        }
        if name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            continue;
        }
        found.insert(name.to_owned());
        if found.len() > MAX_ENCODERS {
            return Err("FFmpeg reports more encoders than the diagnostic budget permits.".into());
        }
    }
    if found.is_empty() {
        return Err(
            "FFmpeg did not report any video encoders. Install a full FFmpeg build and retry."
                .into(),
        );
    }
    Ok(found.into_iter().collect())
}

/// The same production argv encodes one private raw RGBA frame; a codec name alone never enables hardware output.
fn probe(
    path: &Path,
    codec: ExportCodec,
    encoder: Encoder,
    compiled: &[String],
    cancellation: &Cancellation,
) -> EncoderProbe {
    let name = super::options::encoder_name(codec, encoder);
    let result = (|| {
        cancellation.check()?;
        if !compiled.iter().any(|compiled| compiled == name) {
            return Err(format!(
                "This FFmpeg build does not contain {name}. Install a build with that encoder or select another output format."
            ));
        }
        let private = tempfile::Builder::new()
            .prefix("cinnabar-encoder-check-")
            .tempdir()
            .map_err(|error| error.to_string())?;
        let output = private.path().join("probe.mp4");
        let settings = VideoSettings {
            path: output.clone(),
            fps: FrameRate {
                numerator: 1,
                denominator: 1,
            },
            codec,
        };
        let options = EncodeOptions {
            encoder,
            quality: None,
            speed: EncodingSpeed::Live,
        };
        let args = super::options::arguments(
            &settings,
            PROBE_SIZE,
            PixelLayout::Rgba,
            &output,
            options,
            encoder,
        );
        let mut command = Command::new(path);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = MediaChild::start_timed(command, cancellation.clone(), TOOL_TIMEOUT)?;
        let pixels = vec![0; super::frame_byte_len(PROBE_SIZE)?];
        child
            .stdin
            .as_mut()
            .ok_or("The encoder input pipe is unavailable.")?
            .write_all(&pixels)
            .map_err(|error| error.to_string())?;
        child.finish()?;
        let size = std::fs::metadata(&output)
            .map_err(|error| error.to_string())?
            .len();
        if size == 0 {
            return Err("The encoder produced an empty file.".into());
        }
        Ok(())
    })();
    EncoderProbe { encoder: name.into(), available: result.is_ok(), error: result.err().map(|error: String| format!("{name} failed its one-frame capability check ({} s limit): {error}. Update FFmpeg/graphics drivers, retry, or select proven CPU encoding.", TOOL_TIMEOUT.as_secs())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encoder_listing_is_derived_from_video_flags_and_does_not_infer_device_support() {
        let parsed = parse_encoders("Encoders:\n V..... = Video\n V....D libx264 H.264\n V..... h264_amf AMD\n V....D h264_nvenc NVIDIA\n A..... aac Audio\n V..... h264_amf duplicate\n").unwrap();
        assert_eq!(parsed, ["h264_amf", "h264_nvenc", "libx264"]);
        assert!(parse_encoders("A..... aac Audio").is_err());
    }
}
