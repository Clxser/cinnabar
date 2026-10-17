//! Worker-only admission of local media, content identity and bounded thumbnail decoding.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

use crate::{
    FrameRate, Id, MediaTime,
    export::Cancellation,
    media_process::MediaChild,
    model::{MediaKind, MediaSource},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const IMPORT_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "wav", "mp3", "ogg", "flac", "m4a", "png", "jpg", "jpeg",
    "webp", "bmp",
];
pub const THUMBNAIL_SIZE: [u32; 2] = [160, 90];
pub const MAX_THUMBNAILS: usize = 24;
pub const THUMBNAIL_COLUMNS: usize = 6;
pub const MAX_MEDIA_BYTES: u64 = 256 * 1024 * 1024 * 1024;
const MAX_PROBE_BYTES: u64 = 256 * 1024;
const IMAGE_DURATION: MediaTime = MediaTime(5 * crate::time::MICROS_PER_SECOND);

#[derive(Clone, Debug)]
pub struct ProbedMedia {
    pub source: MediaSource,
    pub has_audio: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThumbnailSheet {
    pub size: [u32; 2],
    pub rgba8: Arc<[u8]>,
}

/// Adds one immutable source and its linked tracks as a single reversible transaction.
pub fn import_commands(
    project: &crate::Project,
    mut media: ProbedMedia,
    at: MediaTime,
) -> Result<Vec<crate::edit::Command>, String> {
    use crate::{
        PlaybackRate, Span,
        edit::Command,
        model::{AudioClip, AudioRole, Clip, Framing},
    };
    let source = project.next_id().map_err(|error| error.to_string())?;
    let clip = Id(source
        .0
        .checked_add(1)
        .ok_or("Project identifiers exhausted.")?);
    let audio = Id(source
        .0
        .checked_add(2)
        .ok_or("Project identifiers exhausted.")?);
    media.source.id = source;
    let range =
        Span::new(MediaTime::ZERO, media.source.duration).map_err(|error| error.to_string())?;
    let visual = media.source.kind != MediaKind::Audio;
    let mut commands = vec![Command::AddSource {
        source: media.source,
    }];
    if visual {
        commands.push(Command::AddClip {
            clip: Clip {
                id: clip,
                source,
                source_range: range,
                at,
                rate: PlaybackRate::default(),
                speed_ramp: None,
                track: 0,
                framing: Framing::default(),
                enabled: true,
            },
        });
    }
    if media.has_audio {
        commands.push(Command::AddAudio {
            audio: AudioClip {
                id: audio,
                source,
                linked_clip: visual.then_some(clip),
                source_range: range,
                at,
                rate: PlaybackRate::default(),
                speed_ramp: None,
                role: AudioRole::Music,
                gain: 1.0,
                fade_in: MediaTime::ZERO,
                fade_out: MediaTime::ZERO,
                muted: false,
            },
        });
    }
    Ok(commands)
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
}
#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    #[serde(default)]
    disposition: Disposition,
}
#[derive(Default, Deserialize)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}
#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
    format_name: Option<String>,
}

/// Probes a bounded regular local file. Paths never enter a shell or a network-capable input protocol.
pub fn probe(path: &Path, cancellation: &Cancellation) -> Result<ProbedMedia, String> {
    cancellation.check()?;
    let path =
        std::fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))?;
    check_file(&path)?;
    let ffmpeg = crate::video::find_ffmpeg()?;
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let executable = ffmpeg
        .parent()
        .map(|parent| parent.join(name))
        .filter(|path| path.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|value| {
                std::env::split_paths(&value)
                    .map(|directory| directory.join(name))
                    .find(|path| path.is_file())
            })
        })
        .ok_or("FFprobe was not found. Install it beside FFmpeg to import or recover media.")?;
    let mut command = Command::new(executable);
    command.args(["-v", "error", "-protocol_whitelist", "file,pipe", "-show_entries", "stream=codec_type,width,height,duration,avg_frame_rate,r_frame_rate:stream_disposition=attached_pic:format=duration,format_name", "-of", "json"])
        .arg(&path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child =
        MediaChild::start_timed(command, cancellation.clone(), Duration::from_secs(20))?;
    let mut bytes = Vec::new();
    child
        .stdout
        .as_mut()
        .ok_or("Media probe output is unavailable.")?
        .take(MAX_PROBE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_PROBE_BYTES {
        return Err("Media metadata exceeds its byte limit.".into());
    }
    child.finish()?;
    let probe: Probe = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid media metadata: {error}"))?;
    admit_probe(path, probe)
}

/// Metadata is admitted before importing; content hashing supports exact relink after a file move.
pub fn import(path: &Path, cancellation: &Cancellation) -> Result<ProbedMedia, String> {
    let mut media = probe(path, cancellation)?;
    media.source.sha256 = Some(hash(&media.source.path, cancellation)?);
    Ok(media)
}

/// A moved source must retain its identity, kind, usable extent and source cadence.
pub fn relink(
    old: &MediaSource,
    path: &Path,
    cancellation: &Cancellation,
) -> Result<MediaSource, String> {
    if old.kind == MediaKind::Replay {
        return Err("Relink a replay through its original recording folder.".into());
    }
    let media = import(path, cancellation)?;
    let next = media.source;
    if old.sha256.is_some_and(|hash| Some(hash) != next.sha256)
        || old.kind != next.kind
        || next.duration < old.duration
        || old.size != next.size
        || old
            .frame_rate
            .zip(next.frame_rate)
            .is_some_and(|(old, new)| old != new)
    {
        return Err(
            "The selected file does not match this source's content or media properties.".into(),
        );
    }
    let mut source = old.clone();
    source.path = next.path;
    source.sha256 = next.sha256;
    source.frame_rate = next.frame_rate.or(old.frame_rate);
    Ok(source)
}

/// One small RGB frame is decoded with fixed memory, time and local-protocol limits.
pub fn thumbnail(path: &Path, cancellation: &Cancellation) -> Result<Arc<[u8]>, String> {
    check_file(path)?;
    let mut command = Command::new(crate::video::find_ffmpeg()?);
    let [width, height] = THUMBNAIL_SIZE;
    command.args(["-v", "error", "-nostdin", "-threads", "1", "-protocol_whitelist", "file,pipe", "-i"])
        .arg(path).args(["-map", "0:v:0", "-frames:v", "1", "-vf"])
        .arg(format!("scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"))
        .args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child =
        MediaChild::start_timed(command, cancellation.clone(), Duration::from_secs(10))?;
    let length = (width * height * 4) as usize;
    let mut bytes = Vec::with_capacity(length);
    child
        .stdout
        .as_mut()
        .ok_or("Thumbnail output is unavailable.")?
        .take(length as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() != length {
        return Err("The source has no decodable thumbnail.".into());
    }
    child.finish()?;
    Ok(bytes.into())
}

/// Source bytes are streamed through one small buffer and cancellation is checked between reads.
fn hash(path: &Path, cancellation: &Cancellation) -> Result<[u8; 32], String> {
    let expected = check_file(path)?;
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut bytes = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        cancellation.check()?;
        let count = file.read(&mut bytes).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > expected {
            return Err("The source changed while it was being imported.".into());
        }
        digest.update(&bytes[..count]);
    }
    if total != expected {
        return Err("The source changed while it was being imported.".into());
    }
    Ok(digest.finalize().into())
}

/// Media byte limits apply before FFmpeg receives a filename or import begins hashing.
fn check_file(path: &Path) -> Result<u64, String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_MEDIA_BYTES {
        return Err("Choose a nonempty local media file within the 256 GiB import limit.".into());
    }
    Ok(metadata.len())
}

/// Durations are finite and bounded by the timeline owner, with still images getting an editable default.
fn admit_probe(path: PathBuf, probe: Probe) -> Result<ProbedMedia, String> {
    if probe.streams.len() > 256 {
        return Err("Media contains too many streams.".into());
    }
    let video = probe.streams.iter().find(|stream| {
        stream.codec_type.as_deref() == Some("video") && stream.disposition.attached_pic == 0
    });
    let has_audio = probe
        .streams
        .iter()
        .any(|stream| stream.codec_type.as_deref() == Some("audio"));
    let format = probe.format.as_ref();
    let seconds = video
        .and_then(|stream| stream.duration.as_deref())
        .and_then(|duration| duration.parse::<f64>().ok())
        .or_else(|| {
            format
                .and_then(|format| format.duration.as_deref())
                .and_then(|duration| duration.parse::<f64>().ok())
        });
    let image = video.is_some()
        && format
            .and_then(|format| format.format_name.as_deref())
            .is_some_and(|name| {
                name.split(',')
                    .any(|name| name == "image2" || name.ends_with("_pipe"))
            });
    let kind = if image {
        MediaKind::Image
    } else if video.is_some() {
        MediaKind::Video
    } else if has_audio {
        MediaKind::Audio
    } else {
        return Err("No supported video, image or audio stream was found.".into());
    };
    let duration = if image {
        IMAGE_DURATION
    } else {
        MediaTime::from_seconds(seconds.ok_or("Media duration is unavailable.")?)
            .map_err(|error| error.to_string())?
    };
    if duration == MediaTime::ZERO {
        return Err("Media contains no playable duration.".into());
    }
    let size = video.map_or([0, 0], |stream| {
        [stream.width.unwrap_or(0), stream.height.unwrap_or(0)]
    });
    if video.is_some() && (size.contains(&0) || size.iter().any(|&side| side > 32_768)) {
        return Err("Unsupported media dimensions.".into());
    }
    let frame_rate = if image {
        None
    } else {
        video.and_then(|stream| {
            stream
                .avg_frame_rate
                .as_deref()
                .and_then(rate)
                .or_else(|| stream.r_frame_rate.as_deref().and_then(rate))
        })
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Imported media")
        .to_owned();
    let mut end = name.len().min(256);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let name = name[..end].to_owned();
    Ok(ProbedMedia {
        source: MediaSource {
            id: Id(0),
            name,
            path,
            kind,
            duration,
            size,
            frame_rate,
            sha256: None,
        },
        has_audio: has_audio && !image,
    })
}

/// Only source rates supported by the decoder's rational-rate contract are retained.
fn rate(value: &str) -> Option<FrameRate> {
    let (numerator, denominator) = value.split_once('/')?;
    let fps = FrameRate {
        numerator: numerator.parse().ok()?,
        denominator: denominator.parse().ok()?,
    };
    fps.validate().ok()?;
    Some(fps)
}
