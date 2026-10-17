//! Frozen runtime render assets, including session overlay metadata absent from startup carriers.

use std::{
    borrow::Cow,
    io::{Read, Write},
    sync::atomic::AtomicU64,
};

use block_transform::random_offset::{RandomOffsetAxis, RandomOffsetComponent};
use serde::{Deserialize, Serialize};

use super::RuntimeAssets;
use crate::{
    Animation, AssetError, MAX_ANIMATION_FRAMES, MAX_MODEL_TEMPLATES, TextureRef,
    compiled::CompiledAssetsRef,
};

const MAGIC: [u8; 8] = *b"CIRWLD01";
const HEADER_BYTES: usize = MAGIC.len() + std::mem::size_of::<u64>() + std::mem::size_of::<u32>();
const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata<'a> {
    offsets: Vec<Offset>,
    source_sizes: Cow<'a, [[u16; 2]]>,
    source_grids: Cow<'a, [u8]>,
    shard_layers: Cow<'a, [usize]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Offset {
    template: u32,
    ranges: [[f32; 2]; 3],
    steps: [u32; 3],
}

impl RuntimeAssets {
    /// Encodes exact runtime render tables on an IO worker, borrowing large texture allocations.
    /// Ordinary startup-carrier limits stay unchanged; the recording envelope also retains overlays.
    pub fn write_recording(
        &self,
        output: &mut dyn Write,
        maximum_bytes: usize,
    ) -> Result<(), AssetError> {
        let Self {
            visuals,
            light_properties,
            hashed,
            materials,
            model_templates,
            model_random_offsets,
            model_quads,
            animations,
            animation_frames,
            texture_pages,
            overlay_texture_source_sizes,
            overlay_texture_source_grids,
            overlay_page_layer_counts,
            biomes,
            provenance,
            missing: _,
        } = self;
        let metadata = Metadata {
            offsets: model_random_offsets
                .iter()
                .map(|&(template, offset)| Offset {
                    template,
                    ranges: offset.axes.map(|axis| axis.range),
                    steps: offset.axes.map(|axis| axis.steps),
                })
                .collect(),
            source_sizes: Cow::Borrowed(overlay_texture_source_sizes),
            source_grids: Cow::Borrowed(overlay_texture_source_grids),
            shard_layers: Cow::Borrowed(overlay_page_layer_counts),
        };
        validate_metadata(&metadata, self)?;
        let metadata = serde_json::to_vec(&metadata)
            .map_err(|error| invalid(format!("encode runtime recording metadata: {error}")))?;
        if metadata.len() > MAX_METADATA_BYTES {
            return Err(invalid("runtime recording metadata exceeds its byte limit"));
        }
        let maximum_body = maximum_bytes
            .checked_sub(HEADER_BYTES + metadata.len())
            .ok_or_else(|| invalid("runtime recording byte budget is too small"))?;
        let (recorded_animations, recorded_frames) =
            recording_animation_spans(animations, animation_frames)?;
        let compiled = CompiledAssetsRef {
            visuals,
            light_properties,
            hashed: hashed.entries(),
            materials,
            model_templates,
            model_quads,
            animations: &recorded_animations,
            animation_frames: &recorded_frames,
            texture_pages,
            biomes,
            provenance,
        };
        let body = crate::blob::encode_borrowed(
            &compiled,
            crate::blob::WorldBlobMode::Recording,
            maximum_body,
        )?;
        output
            .write_all(&MAGIC)
            .and_then(|()| output.write_all(&(body.len() as u64).to_le_bytes()))
            .and_then(|()| output.write_all(&(metadata.len() as u32).to_le_bytes()))
            .and_then(|()| output.write_all(&body))
            .and_then(|()| output.write_all(&metadata))
            .map_err(|error| invalid(format!("write runtime recording assets: {error}")))
    }

    /// Verifies the frozen body and overlay cross-references before returning independently owned assets.
    /// The outer recording object store verifies the complete source digest before publishing this value.
    pub fn read_recording(input: &mut dyn Read, maximum_bytes: usize) -> Result<Self, AssetError> {
        let mut header = [0_u8; HEADER_BYTES];
        input
            .read_exact(&mut header)
            .map_err(|_| invalid("truncated frozen runtime asset header"))?;
        if header[..MAGIC.len()] != MAGIC {
            return Err(invalid("unsupported frozen runtime asset schema"));
        }
        let body_length = usize::try_from(u64::from_le_bytes(
            header[8..16]
                .try_into()
                .expect("fixed runtime asset header"),
        ))
        .map_err(|_| invalid("frozen runtime body exceeds this platform"))?;
        let metadata_length = u32::from_le_bytes(
            header[16..20]
                .try_into()
                .expect("fixed runtime metadata header"),
        ) as usize;
        if metadata_length > MAX_METADATA_BYTES
            || body_length
                .checked_add(metadata_length)
                .and_then(|bytes| bytes.checked_add(HEADER_BYTES))
                .is_none_or(|bytes| bytes > maximum_bytes)
        {
            return Err(invalid("frozen runtime assets exceed their byte limit"));
        }
        let mut body = vec![0; body_length];
        input
            .read_exact(&mut body)
            .map_err(|_| invalid("truncated frozen runtime body"))?;
        let (mut assets, _) = Self::decode_with_mode(&body, crate::blob::WorldBlobMode::Recording)?;
        drop(body);
        let mut metadata = vec![0; metadata_length];
        input
            .read_exact(&mut metadata)
            .map_err(|_| invalid("truncated frozen runtime metadata"))?;
        let metadata: Metadata<'_> = serde_json::from_slice(&metadata)
            .map_err(|error| invalid(format!("decode frozen runtime metadata: {error}")))?;
        validate_metadata(&metadata, &assets)?;
        assets.model_random_offsets = metadata
            .offsets
            .iter()
            .map(|offset| {
                (
                    offset.template,
                    RandomOffsetComponent {
                        axes: std::array::from_fn(|axis| RandomOffsetAxis {
                            range: offset.ranges[axis],
                            steps: offset.steps[axis],
                        }),
                    },
                )
            })
            .collect();
        assets.overlay_texture_source_sizes = metadata.source_sizes.into_owned().into_boxed_slice();
        assets.overlay_texture_source_grids = metadata.source_grids.into_owned().into_boxed_slice();
        assets.overlay_page_layer_counts = metadata.shard_layers.into_owned().into_boxed_slice();
        assets.missing = AtomicU64::new(0);
        let mut trailing = [0_u8; 1];
        if input
            .read(&mut trailing)
            .map_err(|_| invalid("cannot finish frozen runtime asset object"))?
            != 0
        {
            return Err(invalid("frozen runtime asset object has trailing bytes"));
        }
        Ok(assets)
    }

    /// Conservative pin charge for handing immutable runtime inputs to a bounded recording worker.
    pub fn recording_retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(&*self.visuals)
            + std::mem::size_of_val(&*self.light_properties)
            + self.hashed.retained_bytes()
            + std::mem::size_of_val(&*self.materials)
            + std::mem::size_of_val(&*self.model_templates)
            + std::mem::size_of_val(&*self.model_random_offsets)
            + std::mem::size_of_val(&*self.model_quads)
            + std::mem::size_of_val(&*self.animations)
            + std::mem::size_of_val(&*self.animation_frames)
            + std::mem::size_of_val(&*self.texture_pages)
            + self
                .texture_pages
                .iter()
                .map(|page| std::mem::size_of_val(&*page.texture.mips))
                .sum::<usize>()
            + self
                .texture_pages
                .iter()
                .flat_map(|page| page.texture.mips.iter())
                .map(|mip| mip.rgba8.len())
                .sum::<usize>()
            + std::mem::size_of_val(&*self.overlay_texture_source_sizes)
            + std::mem::size_of_val(&*self.overlay_texture_source_grids)
            + std::mem::size_of_val(&*self.overlay_page_layer_counts)
            + self.biomes.tint_maps_rgb8.len()
            + std::mem::size_of_val(&*self.biomes.rules)
            + self
                .biomes
                .rules
                .iter()
                .map(|rule| rule.name.len())
                .sum::<usize>()
    }
}

/// Runtime overlays may share or reorder frame spans; the frozen carrier owns contiguous copies.
/// Animation indices, frame order and timing stay unchanged, and startup carrier validation stays strict.
fn recording_animation_spans<'a>(
    animations: &'a [Animation],
    frames: &'a [TextureRef],
) -> Result<(Cow<'a, [Animation]>, Cow<'a, [TextureRef]>), AssetError> {
    let mut count = 0_usize;
    let mut contiguous = true;
    for animation in animations {
        let start = animation.frame_start as usize;
        let end = start
            .checked_add(animation.frame_count as usize)
            .filter(|&end| end <= frames.len())
            .ok_or_else(|| invalid("recorded animation refers outside its runtime frame table"))?;
        contiguous &= start == count;
        count = count
            .checked_add(end - start)
            .filter(|&count| count <= MAX_ANIMATION_FRAMES)
            .ok_or_else(|| invalid("recorded animation frames exceed their byte budget"))?;
    }
    if contiguous && count == frames.len() {
        return Ok((Cow::Borrowed(animations), Cow::Borrowed(frames)));
    }
    let mut recorded = Vec::with_capacity(animations.len());
    let mut packed = Vec::with_capacity(count);
    for animation in animations {
        let start = animation.frame_start as usize;
        recorded.push(Animation {
            frame_start: packed.len() as u32,
            ..*animation
        });
        packed.extend_from_slice(&frames[start..start + animation.frame_count as usize]);
    }
    Ok((Cow::Owned(recorded), Cow::Owned(packed)))
}

/// Reuses overlay source admission and the authored random-offset owner's validation contract.
fn validate_metadata(metadata: &Metadata<'_>, assets: &RuntimeAssets) -> Result<(), AssetError> {
    if metadata.offsets.len() > MAX_MODEL_TEMPLATES
        || metadata
            .offsets
            .windows(2)
            .any(|pair| pair[0].template >= pair[1].template)
        || metadata.offsets.iter().any(|offset| {
            offset.template as usize >= assets.model_templates.len()
                || !(RandomOffsetComponent {
                    axes: std::array::from_fn(|axis| RandomOffsetAxis {
                        range: offset.ranges[axis],
                        steps: offset.steps[axis],
                    }),
                })
                .is_valid()
        })
    {
        return Err(invalid("frozen runtime model offsets are invalid"));
    }
    if metadata.shard_layers.is_empty() {
        if !metadata.source_sizes.is_empty() || !metadata.source_grids.is_empty() {
            return Err(invalid("frozen runtime UV metadata has no overlay shards"));
        }
        return Ok(());
    }
    let pages = assets
        .texture_pages
        .get(1..)
        .ok_or_else(|| invalid("frozen runtime overlay has no base page"))?;
    if metadata.shard_layers.len() != pages.len()
        || metadata
            .shard_layers
            .iter()
            .zip(pages)
            .any(|(&count, page)| count != page.texture.layers as usize)
    {
        return Err(invalid(
            "frozen runtime overlay layer counts do not match their texture pages",
        ));
    }
    super::overlay::validate_source_metadata(
        pages.iter().map(|page| &page.texture),
        &metadata.source_sizes,
        &metadata.source_grids,
    )
}

/// Runtime recording failures retain their original meaning without claiming a startup carrier failed.
fn invalid(detail: impl Into<Box<str>>) -> AssetError {
    AssetError::InvalidCompiledAssets {
        detail: detail.into(),
    }
}
