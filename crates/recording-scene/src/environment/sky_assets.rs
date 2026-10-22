//! Content-addressed atmosphere and optional pack skyboxes, shared by frame records.

use std::{
    io::{Read, Write},
    sync::Arc,
};

use assets::{PackSkybox, RuntimeAtmosphereAssets, SKYBOX_FACE_COUNT, SkyboxDimension, SkyboxFace};
use recording_studio::{
    archive::ArchiveError,
    object_store::{AssetId, AssetReader, AssetWriter},
};
use serde::{Deserialize, Serialize};

use super::{MAX_ASSET_BYTES, Skyboxes, asset_error, io_error};

const SKYBOX_MAGIC: [u8; 8] = *b"CINSKY01";
const MAX_BINDING_BYTES: u64 = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    atmosphere: AssetId,
    skyboxes: Option<AssetId>,
}

pub(super) struct LoadedAssets {
    pub atmosphere: Arc<RuntimeAtmosphereAssets>,
    pub skyboxes: Option<Arc<Skyboxes>>,
}

/// All large byte writes happen on the archive worker before a frame's small binding is published.
pub(super) fn store(
    store: &mut AssetWriter,
    atmosphere: &RuntimeAtmosphereAssets,
    skyboxes: Option<&Skyboxes>,
) -> Result<AssetId, ArchiveError> {
    let atmosphere = store.put(MAX_ASSET_BYTES, |out| {
        atmosphere
            .write_recording(out, MAX_ASSET_BYTES as usize)
            .map_err(asset_error)
    })?;
    let skyboxes = skyboxes
        .map(|skyboxes| store.put(MAX_ASSET_BYTES, |out| write_skyboxes(out, skyboxes)))
        .transpose()?;
    let bytes = serde_json::to_vec(&Binding {
        atmosphere,
        skyboxes,
    })?;
    store.put(MAX_BINDING_BYTES, |out| {
        out.write_all(&bytes).map_err(io_error)
    })
}

/// Each object is checksum-admitted before the complete binding is returned to a staging scene.
pub(super) fn load(
    store: &AssetReader,
    id: AssetId,
    cancelled: &impl Fn() -> bool,
) -> Result<LoadedAssets, ArchiveError> {
    let binding: Binding = store.read_cancellable(id, MAX_BINDING_BYTES, cancelled, |input| {
        Ok(serde_json::from_reader(input)?)
    })?;
    let atmosphere =
        store.read_cancellable(binding.atmosphere, MAX_ASSET_BYTES, cancelled, |input| {
            RuntimeAtmosphereAssets::read_recording(input, MAX_ASSET_BYTES as usize)
                .map(Arc::new)
                .map_err(asset_error)
        })?;
    let skyboxes = binding
        .skyboxes
        .map(|id| {
            store.read_cancellable(id, MAX_ASSET_BYTES, cancelled, |input| {
                read_skyboxes(input).map(Arc::new)
            })
        })
        .transpose()?;
    Ok(LoadedAssets {
        atmosphere,
        skyboxes,
    })
}

/// Texture references are positional in the owner's dimension/face order; there are no arbitrary paths.
fn write_skyboxes(output: &mut dyn Write, skyboxes: &Skyboxes) -> Result<(), ArchiveError> {
    let mut remaining = MAX_ASSET_BYTES as usize - SKYBOX_MAGIC.len();
    output.write_all(&SKYBOX_MAGIC).map_err(io_error)?;
    for dimension in SkyboxDimension::ALL {
        let PackSkybox { faces } = &skyboxes[dimension as usize];
        for face in faces {
            remaining = remaining.checked_sub(4).ok_or(ArchiveError::Quota)?;
            match face {
                None => output.write_all(&0_u32.to_le_bytes()).map_err(io_error)?,
                Some(face) => {
                    let SkyboxFace {
                        width,
                        height,
                        rgba8,
                    } = face;
                    let bytes = pixel_bytes(*width, *height)?;
                    remaining = remaining
                        .checked_sub(8 + bytes)
                        .ok_or(ArchiveError::Quota)?;
                    if bytes != rgba8.len() {
                        return Err(ArchiveError::Invalid(
                            "recorded skybox pixels do not match their size",
                        ));
                    }
                    for value in [1_u32, *width, *height] {
                        output.write_all(&value.to_le_bytes()).map_err(io_error)?;
                    }
                    output.write_all(rgba8).map_err(io_error)?;
                }
            }
        }
    }
    Ok(())
}

/// Byte admission precedes every raster allocation, with one cumulative budget for all dimensions.
fn read_skyboxes(input: &mut dyn Read) -> Result<Skyboxes, ArchiveError> {
    let mut magic = [0; SKYBOX_MAGIC.len()];
    input.read_exact(&mut magic).map_err(io_error)?;
    if magic != SKYBOX_MAGIC {
        return Err(ArchiveError::Invalid("unsupported recorded skybox schema"));
    }
    let mut remaining = MAX_ASSET_BYTES as usize - magic.len();
    let mut skyboxes: Skyboxes = std::array::from_fn(|_| PackSkybox::default());
    for dimension in SkyboxDimension::ALL {
        for index in 0..SKYBOX_FACE_COUNT {
            remaining = remaining.checked_sub(4).ok_or(ArchiveError::Quota)?;
            match read_u32(input)? {
                0 => {}
                1 => {
                    let width = read_u32(input)?;
                    let height = read_u32(input)?;
                    let bytes = pixel_bytes(width, height)?;
                    remaining = remaining
                        .checked_sub(8 + bytes)
                        .ok_or(ArchiveError::Quota)?;
                    let mut pixels = vec![0; bytes];
                    input.read_exact(&mut pixels).map_err(io_error)?;
                    skyboxes[dimension as usize].faces[index] = Some(SkyboxFace {
                        width,
                        height,
                        rgba8: pixels.into_boxed_slice(),
                    });
                }
                _ => {
                    return Err(ArchiveError::Invalid(
                        "invalid recorded skybox presence flag",
                    ));
                }
            }
        }
    }
    let mut trailing = [0];
    if input.read(&mut trailing).map_err(io_error)? != 0 {
        return Err(ArchiveError::Invalid(
            "recorded skybox object has trailing bytes",
        ));
    }
    Ok(skyboxes)
}

/// Image dimensions cannot overflow allocation arithmetic or independently exhaust the object budget.
fn pixel_bytes(width: u32, height: u32) -> Result<usize, ArchiveError> {
    if width == 0 || height == 0 {
        return Err(ArchiveError::Invalid(
            "recorded skybox has an empty dimension",
        ));
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|&bytes| bytes <= MAX_ASSET_BYTES as usize)
        .ok_or(ArchiveError::Quota)
}

/// All scalar fields use explicit little endian rather than native struct layout.
fn read_u32(input: &mut dyn Read) -> Result<u32, ArchiveError> {
    let mut bytes = [0; 4];
    input.read_exact(&mut bytes).map_err(io_error)?;
    Ok(u32::from_le_bytes(bytes))
}
