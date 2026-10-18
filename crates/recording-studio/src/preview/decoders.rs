//! Simultaneous clip instances retain independent decode cursors, even when they share one file.

use super::*;

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
