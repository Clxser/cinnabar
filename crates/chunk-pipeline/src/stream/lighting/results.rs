//! Prepares light change and monotonicity evidence from immutable worker inputs.

use super::super::*;
use std::sync::Weak;

/// Monotonic faces are valid only for these exact prior light and provenance snapshots.
#[derive(Debug, Default)]
pub(in crate::stream) struct PreparedMonotonicFaces {
    previous_light: Option<Weak<SubChunkLight>>,
    previous_direct: Option<(u64, Weak<DirectSkyMask>)>,
    faces: [bool; 6],
}

impl PreparedMonotonicFaces {
    /// Compares each boundary once while retaining the prior identities needed at acceptance.
    pub(in crate::stream) fn prepare(
        previous_light: Option<Arc<SubChunkLight>>,
        previous_direct: Option<StoredDirectSky>,
        replacement: &SubChunkLight,
        replacement_direct: &DirectSkyMask,
    ) -> ([bool; 6], Self) {
        let (changed, faces) = compare_faces(
            previous_light.as_deref(),
            previous_direct.as_ref(),
            replacement,
            replacement_direct,
        );
        (
            changed,
            Self::from_compared(previous_light, previous_direct, faces),
        )
    }

    /// Retains allocation identities without keeping retired light or mask payloads alive.
    fn from_compared(
        light: Option<Arc<SubChunkLight>>,
        direct: Option<StoredDirectSky>,
        faces: [bool; 6],
    ) -> Self {
        Self {
            previous_light: light.as_ref().map(Arc::downgrade),
            previous_direct: direct
                .map(|direct| (direct.light_revision, Arc::downgrade(&direct.mask))),
            faces,
        }
    }

    /// Returns worker evidence only while both current snapshots retain their captured identities.
    pub(in crate::stream) fn matching(
        self,
        light: Option<&Arc<SubChunkLight>>,
        direct: Option<&StoredDirectSky>,
    ) -> Option<[bool; 6]> {
        let same_light = match (&self.previous_light, light) {
            (None, None) => true,
            (Some(before), Some(current)) => std::ptr::eq(before.as_ptr(), Arc::as_ptr(current)),
            _ => false,
        };
        let same_direct = match (&self.previous_direct, direct) {
            (None, None) => true,
            (Some((revision, mask)), Some(current)) => {
                *revision == current.light_revision
                    && std::ptr::eq(mask.as_ptr(), Arc::as_ptr(&current.mask))
            }
            _ => false,
        };
        (same_light && same_direct).then_some(self.faces)
    }
}

/// Captured target inputs shared by single-section and column-batch result preparation.
pub(in crate::stream) struct LightBatchTarget {
    pub(super) key: SubChunkKey,
    pub(super) identity: LightJobIdentity,
    pub(super) queued_at: Instant,
    old_light: Option<Arc<SubChunkLight>>,
    old_direct: Option<StoredDirectSky>,
}

impl LightBatchTarget {
    /// Keeps raw prior provenance so an invalid generation cannot become an absent record.
    pub(in crate::stream) fn capture(job: &PreparedLightJob) -> Self {
        Self {
            key: job.key,
            identity: job.identity,
            queued_at: job.queued_at,
            old_light: job.prior.light.light(job.key).cloned(),
            old_direct: job.prior.direct_sky.get(&job.key).cloned(),
        }
    }

    /// Prepares publication changes and monotonic faces before the worker releases its result.
    pub(in crate::stream) fn finish(
        self,
        replacement: SubChunkLight,
        direct_sky: Arc<DirectSkyMask>,
        used_uniform_fast_path: bool,
    ) -> SolvedLightJob {
        let light_levels_changed = self
            .old_light
            .as_deref()
            .is_none_or(|previous| !light_levels_equal(previous, &replacement));
        let valid_direct = self.old_direct.as_ref().filter(|direct| {
            self.old_light
                .as_deref()
                .is_some_and(|light| direct.light_revision == light.generation())
        });
        let direct_sky_changed =
            valid_direct.is_none_or(|previous| previous.mask.as_ref() != direct_sky.as_ref());
        let (changed_faces, monotonic_faces) = if !light_levels_changed && !direct_sky_changed {
            // Exact equality includes valid prior provenance, so no boundary can differ or decrease.
            (
                [false; 6],
                PreparedMonotonicFaces::from_compared(self.old_light, self.old_direct, [true; 6]),
            )
        } else {
            PreparedMonotonicFaces::prepare(
                self.old_light,
                self.old_direct,
                &replacement,
                direct_sky.as_ref(),
            )
        };
        SolvedLightJob {
            replacement,
            direct_sky,
            used_uniform_fast_path,
            light_levels_changed,
            direct_sky_changed,
            changed_faces,
            monotonic_faces,
        }
    }
}

/// Tracks the two independent stopping conditions during one boundary traversal.
#[derive(Clone, Copy)]
struct FaceComparison {
    changed: bool,
    monotonic: bool,
}

/// Preserves the prior-provenance rules while sharing changed and monotonic cell reads.
fn compare_faces(
    previous: Option<&SubChunkLight>,
    raw_direct: Option<&StoredDirectSky>,
    replacement: &SubChunkLight,
    replacement_direct: &DirectSkyMask,
) -> ([bool; 6], [bool; 6]) {
    let can_prove_monotonic = previous.is_some_and(|light| match raw_direct {
        Some(direct) => direct.light_revision == light.generation(),
        None => {
            light.channel(LightChannel::Sky).is_uniform()
                && light.get(LightChannel::Sky, 0, 0, 0) == Some(0)
        }
    });
    let previous_direct = raw_direct
        .filter(|direct| previous.is_some_and(|light| direct.light_revision == light.generation()))
        .map(|direct| direct.mask.as_ref());
    let initial = FaceComparison {
        changed: false,
        monotonic: can_prove_monotonic,
    };
    let uniform = [LightChannel::Block, LightChannel::Sky]
        .into_iter()
        .all(|channel| {
            previous.is_none_or(|light| light.channel(channel).is_uniform())
                && replacement.channel(channel).is_uniform()
        })
        && previous_direct.is_none_or(|mask| matches!(mask, DirectSkyMask::Uniform(_)))
        && matches!(replacement_direct, DirectSkyMask::Uniform(_));
    let faces = if uniform {
        let mut result = initial;
        compare_cell(
            previous,
            previous_direct,
            replacement,
            replacement_direct,
            [0; 3],
            &mut result,
        );
        [result; 6]
    } else {
        std::array::from_fn(|face| {
            let mut result = initial;
            for position in sampled_light_shell(LIGHT_NEIGHBOUR_OFFSETS[face]) {
                #[cfg(test)]
                lighting::work::record(1, 0);
                compare_cell(
                    previous,
                    previous_direct,
                    replacement,
                    replacement_direct,
                    position,
                    &mut result,
                );
                if result.changed && !result.monotonic {
                    break;
                }
            }
            result
        })
    };
    (
        faces.map(|result| result.changed),
        faces.map(|result| result.monotonic),
    )
}

/// Stops reading a cell once both its face change and loss of monotonicity are known.
fn compare_cell(
    previous: Option<&SubChunkLight>,
    previous_direct: Option<&DirectSkyMask>,
    replacement: &SubChunkLight,
    replacement_direct: &DirectSkyMask,
    [x, y, z]: [u8; 3],
    result: &mut FaceComparison,
) {
    for channel in [LightChannel::Block, LightChannel::Sky] {
        #[cfg(test)]
        lighting::work::record(0, 1);
        let before = previous
            .and_then(|light| light.get(channel, x, y, z))
            .unwrap_or(0);
        let after = replacement
            .get(channel, x, y, z)
            .expect("face coordinates are in range");
        if !result.changed {
            result.changed = before != after;
        }
        if result.monotonic {
            result.monotonic = before <= after;
        }
        if result.changed && !result.monotonic {
            return;
        }
    }
    let before = previous_direct.is_some_and(|direct| direct.get(x, y, z));
    if !result.changed || (result.monotonic && before) {
        let after = replacement_direct.get(x, y, z);
        result.changed |= before != after;
        result.monotonic &= !before || after;
    }
}
