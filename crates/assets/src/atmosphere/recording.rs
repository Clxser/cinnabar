//! Frozen runtime atmosphere inputs, including pack replacements outside startup texture dimensions.

use std::{
    borrow::Cow,
    io::{Read, Write},
};

use super::*;

const MAGIC: [u8; 8] = *b"CINATM01";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata<'a> {
    carrier_identity: [u8; 32],
    source_manifest_sha256: [u8; 32],
    textures: Vec<TextureMetadata<'a>>,
    biomes: Cow<'a, [BiomeVisualProfile]>,
    fogs: Cow<'a, [FogProfile]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextureMetadata<'a> {
    role: u32,
    source_path: Cow<'a, str>,
    source_bytes: u32,
    source_sha256: [u8; 32],
    pixels_sha256: [u8; 32],
    size: [u32; 2],
}

impl RuntimeAtmosphereAssets {
    /// Borrows live textures and profiles while encoding a bounded independent scene asset on a worker.
    pub fn write_recording(
        &self,
        output: &mut dyn Write,
        maximum: usize,
    ) -> Result<(), AssetError> {
        let Self {
            carrier_identity,
            source_manifest_sha256,
            textures,
            biome_profiles,
            fog_profiles,
        } = self;
        let metadata = Metadata {
            carrier_identity: *carrier_identity,
            source_manifest_sha256: *source_manifest_sha256,
            textures: textures
                .iter()
                .map(|texture| {
                    let AtmosphereTexture {
                        role,
                        source_path,
                        source_bytes,
                        source_sha256,
                        pixels_sha256,
                        width,
                        height,
                        rgba8: _,
                    } = texture;
                    TextureMetadata {
                        role: *role as u32,
                        source_path: Cow::Borrowed(source_path),
                        source_bytes: *source_bytes,
                        source_sha256: *source_sha256,
                        pixels_sha256: *pixels_sha256,
                        size: [*width, *height],
                    }
                })
                .collect(),
            biomes: Cow::Borrowed(biome_profiles),
            fogs: Cow::Borrowed(fog_profiles),
        };
        let payload = validate(&metadata, maximum)?;
        if textures
            .iter()
            .zip(&metadata.textures)
            .any(|(texture, meta)| {
                pixel_length(meta.size[0], meta.size[1]).ok() != Some(texture.rgba8.len())
            })
        {
            return Err(invalid(
                "frozen atmosphere pixels do not match their dimensions",
            ));
        }
        let metadata = serde_json::to_vec(&metadata)
            .map_err(|_| invalid("cannot encode frozen atmosphere metadata"))?;
        if metadata.len() > MAX_ENVIRONMENT_SECTION_BYTES
            || payload
                .checked_add(metadata.len() + MAGIC.len() + 4)
                .is_none_or(|bytes| bytes > maximum)
        {
            return Err(invalid("frozen atmosphere object exceeds its byte limit"));
        }
        output
            .write_all(&MAGIC)
            .and_then(|()| output.write_all(&(metadata.len() as u32).to_le_bytes()))
            .and_then(|()| output.write_all(&metadata))
            .map_err(|_| invalid("cannot write frozen atmosphere header"))?;
        for texture in textures {
            output
                .write_all(&texture.rgba8)
                .map_err(|_| invalid("cannot write frozen atmosphere pixels"))?;
        }
        Ok(())
    }

    /// Preserves runtime replacements while leaving ordinary startup-carrier validation unchanged.
    pub fn read_recording(input: &mut dyn Read, maximum: usize) -> Result<Self, AssetError> {
        let mut magic = [0; MAGIC.len()];
        input
            .read_exact(&mut magic)
            .map_err(|_| invalid("truncated frozen atmosphere header"))?;
        let mut length = [0; 4];
        input
            .read_exact(&mut length)
            .map_err(|_| invalid("truncated frozen atmosphere metadata length"))?;
        let length = u32::from_le_bytes(length) as usize;
        if magic != MAGIC
            || length > MAX_ENVIRONMENT_SECTION_BYTES
            || length + MAGIC.len() + 4 > maximum
        {
            return Err(invalid("unsupported or oversized frozen atmosphere object"));
        }
        let mut bytes = vec![0; length];
        input
            .read_exact(&mut bytes)
            .map_err(|_| invalid("truncated frozen atmosphere metadata"))?;
        let metadata: Metadata<'_> = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("invalid frozen atmosphere metadata"))?;
        let remaining = maximum - length - MAGIC.len() - 4;
        validate(&metadata, remaining)?;
        let mut textures = Vec::with_capacity(metadata.textures.len());
        for texture in metadata.textures {
            let mut rgba8 = vec![0; pixel_length(texture.size[0], texture.size[1])?];
            input
                .read_exact(&mut rgba8)
                .map_err(|_| invalid("truncated frozen atmosphere pixels"))?;
            let role = role(texture.role)
                .ok_or_else(|| invalid("unknown frozen atmosphere texture role"))?;
            textures.push(AtmosphereTexture {
                role,
                source_path: texture.source_path.into_owned().into_boxed_str(),
                source_bytes: texture.source_bytes,
                source_sha256: texture.source_sha256,
                pixels_sha256: texture.pixels_sha256,
                width: texture.size[0],
                height: texture.size[1],
                rgba8: rgba8.into_boxed_slice(),
            });
        }
        let mut trailing = [0; 1];
        if input
            .read(&mut trailing)
            .map_err(|_| invalid("cannot finish frozen atmosphere object"))?
            != 0
        {
            return Err(invalid("frozen atmosphere object has trailing bytes"));
        }
        Ok(Self {
            carrier_identity: metadata.carrier_identity,
            source_manifest_sha256: metadata.source_manifest_sha256,
            textures: textures.into_boxed_slice(),
            biome_profiles: metadata.biomes.into_owned().into_boxed_slice(),
            fog_profiles: metadata.fogs.into_owned().into_boxed_slice(),
        })
    }

    /// Charges native pixel and profile ownership while capture workers retain a material epoch.
    pub fn recording_retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(&*self.textures)
            + self
                .textures
                .iter()
                .map(|texture| texture.rgba8.len() + texture.source_path.len())
                .sum::<usize>()
            + std::mem::size_of_val(&*self.biome_profiles)
            + std::mem::size_of_val(&*self.fog_profiles)
            + self
                .biome_profiles
                .iter()
                .map(|profile| {
                    profile.biome_identifier.len()
                        + profile.fog_identifier.len()
                        + profile.atmosphere_identifier.len()
                        + profile.lighting_identifier.len()
                })
                .sum::<usize>()
            + self
                .fog_profiles
                .iter()
                .map(|profile| {
                    profile.identifier.len() + std::mem::size_of_val(&*profile.distances)
                })
                .sum::<usize>()
    }
}

/// Runtime texture admission matches the existing replacement owner, rather than fixed carrier sizes.
fn validate(metadata: &Metadata<'_>, maximum: usize) -> Result<usize, AssetError> {
    if metadata.source_manifest_sha256 == [0; 32]
        || metadata.carrier_identity == [0; 32]
        || metadata.textures.len() != AtmosphereRole::ALL.len()
    {
        return Err(invalid(
            "frozen atmosphere provenance or texture roles are incomplete",
        ));
    }
    validate_environment_profiles(&metadata.biomes, &metadata.fogs)?;
    let mut total = 0_usize;
    for (index, texture) in metadata.textures.iter().enumerate() {
        if role(texture.role).is_none()
            || metadata.textures[..index]
                .iter()
                .any(|old| old.role == texture.role)
            || texture.source_path.is_empty()
            || texture.source_path.len() > MAX_ENVIRONMENT_IDENTIFIER_BYTES * 4
            || texture.size.contains(&0)
        {
            return Err(invalid("invalid frozen atmosphere texture metadata"));
        }
        let bytes = pixel_length(texture.size[0], texture.size[1])?;
        if bytes > MAX_SOURCE_BYTES * 4 {
            return Err(invalid("frozen atmosphere texture exceeds runtime limits"));
        }
        total = total
            .checked_add(bytes)
            .filter(|&bytes| bytes <= maximum)
            .ok_or_else(|| invalid("frozen atmosphere object exceeds its byte limit"))?;
    }
    Ok(total)
}

/// The owner enum is the single source of texture discriminants.
fn role(value: u32) -> Option<AtmosphereRole> {
    AtmosphereRole::ALL
        .into_iter()
        .find(|role| *role as u32 == value)
}
