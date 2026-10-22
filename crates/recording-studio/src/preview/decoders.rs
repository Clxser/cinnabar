//! Simultaneous clip instances retain independent decode cursors, even when they share one file.

use super::*;

/// A non-overlapping cut of unchanged media keeps its decoder and scrub window; overlaps retain independent cursors.
pub(super) fn reuse_cut(
    retained: &mut BTreeMap<Id, VideoDecoder>,
    active: &BTreeSet<Id>,
    clip: Id,
    path: &std::path::Path,
    size: [u32; 2],
    fps: crate::FrameRate,
) {
    if retained.contains_key(&clip) {
        return;
    }
    let previous = retained.iter().find_map(|(&id, decoder)| {
        (!active.contains(&id) && decoder.matches(path, size, fps)).then_some(id)
    });
    if let Some(previous) = previous
        && let Some(decoder) = retained.remove(&previous)
    {
        retained.insert(clip, decoder);
    }
}

/// Opens only deferred configuration; actual media IO stays in the worker's frame request.
pub(super) fn for_clip<'a>(
    retained: &'a mut BTreeMap<Id, VideoDecoder>,
    clip: Id,
    path: &std::path::Path,
    size: [u32; 2],
    fps: crate::FrameRate,
    interactive: bool,
) -> Result<&'a mut VideoDecoder, String> {
    if retained
        .get(&clip)
        .is_none_or(|decoder| !decoder.matches(path, size, fps))
    {
        retained.insert(
            clip,
            if interactive {
                VideoDecoder::interactive(path, size, fps)?
            } else {
                VideoDecoder::new(path, size, fps)?
            },
        );
    }
    retained
        .get_mut(&clip)
        .ok_or_else(|| "Preview decoder is missing.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_cut_reuses_decoded_pixels_even_after_the_source_is_closed() {
        if crate::video::find_ffmpeg().is_err() {
            eprintln!("missing-fixture: FFmpeg executable for adjacent-cut decode reuse");
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("original.ppm");
        let mut pixels = b"P6\n2 2\n255\n".to_vec();
        pixels.extend([255, 0, 0].repeat(4));
        std::fs::write(&path, pixels).unwrap();
        let fps = crate::FrameRate::default();
        let mut retained = BTreeMap::new();
        let original = for_clip(&mut retained, Id(1), &path, [2, 2], fps, false).unwrap();
        let first = original.frame(MediaTime::ZERO, || false).unwrap();
        original.seek();
        std::fs::remove_file(&path).unwrap();
        reuse_cut(
            &mut retained,
            &BTreeSet::from([Id(2)]),
            Id(2),
            &path,
            [2, 2],
            fps,
        );
        assert!(!retained.contains_key(&Id(1)));
        let next = for_clip(&mut retained, Id(2), &path, [2, 2], fps, false).unwrap();
        let reused = next.frame(MediaTime::ZERO, || false).unwrap();
        assert_eq!(first.rgba8, reused.rgba8);
    }

    #[test]
    fn crossfade_instances_of_one_file_have_independent_retained_decoders() {
        let mut retained = BTreeMap::new();
        let path = std::path::Path::new("deferred-local-video.mp4");
        for clip in [Id(1), Id(2)] {
            for_clip(
                &mut retained,
                clip,
                path,
                [16, 8],
                crate::FrameRate::default(),
                true,
            )
            .unwrap();
        }
        assert_eq!(retained.len(), 2);
        reuse_cut(
            &mut retained,
            &BTreeSet::from([Id(1), Id(2), Id(3)]),
            Id(3),
            path,
            [16, 8],
            crate::FrameRate::default(),
        );
        assert!(!retained.contains_key(&Id(3)));
        assert_eq!(retained.len(), 2);
        for_clip(
            &mut retained,
            Id(1),
            path,
            [8, 4],
            crate::FrameRate::default(),
            true,
        )
        .unwrap();
        assert!(
            retained
                .get(&Id(1))
                .unwrap()
                .matches(path, [8, 4], crate::FrameRate::default())
        );
        assert!(
            retained
                .get(&Id(2))
                .unwrap()
                .matches(path, [16, 8], crate::FrameRate::default())
        );
    }
}
