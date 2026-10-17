//! The pinned GeyserMC source: the manifest at `assets::facts::java_mapping::SOURCE_MANIFEST`
//! names a mappings-generator release archive (the published output of the pinned mappings
//! commit) by URL and SHA-256, and the SHA-256 of every file the generator reads from it.

use super::Result;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// The fields the generator reads; provenance fields (repository, commit, editions, release)
/// document the pin for reviewers and the notice.
#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    license: License,
    archive: Archive,
    files: Files,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct License {
    spdx: String,
    path: String,
    sha256: String,
}

#[derive(Deserialize)]
struct Archive {
    url: String,
    sha256: String,
    size_bytes: u64,
    cache: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Files {
    blocks: File,
    java_blocks: File,
    block_tags: File,
    biomes: File,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    path: String,
    sha256: String,
}

/// The verified source files the generator reads.
pub struct SourceFiles {
    /// `blocks_debug.json`: Java block state to Bedrock block state.
    pub blocks: Vec<u8>,
    /// `Blocks.java`: Java blocks in registry order.
    pub java_blocks: String,
    /// `networkTags.nbt`: Java registry tags by registry id.
    pub block_tags: Vec<u8>,
    /// `biomes.json`: Java biome to Bedrock biome id.
    pub biomes: Vec<u8>,
}

/// Reads and verifies every pinned file, from `archive` or from the manifest's cache below
/// `root`, downloading the cache when it is missing or stale.
pub(super) fn load(root: &Path, manifest: &[u8], archive: Option<&Path>) -> Result<SourceFiles> {
    let manifest: Manifest = serde_json::from_slice(manifest)?;
    if manifest.schema != 1 || manifest.license.spdx != "MIT" {
        return Err("unsupported Geyser mappings source manifest".into());
    }
    let bytes = match archive {
        Some(path) => read_verified(path, &manifest.archive)?
            .ok_or_else(|| format!("{} does not match the pinned archive", path.display()))?,
        None => {
            let cache = root.join(&manifest.archive.cache);
            match read_verified(&cache, &manifest.archive)? {
                Some(bytes) => bytes,
                None => download(&manifest.archive, &cache)?,
            }
        }
    };
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut extract = |path: &str, sha256: &str| -> Result<Vec<u8>> {
        let mut entry = zip.by_name(path)?;
        let mut bytes = Vec::new();
        (&mut entry)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("{path} exceeds its size limit").into());
        }
        if format!("{:x}", Sha256::digest(&bytes)) != sha256 {
            return Err(format!("{path} does not match its pinned SHA-256").into());
        }
        Ok(bytes)
    };
    extract(&manifest.license.path, &manifest.license.sha256)?;
    let files = &manifest.files;
    Ok(SourceFiles {
        blocks: extract(&files.blocks.path, &files.blocks.sha256)?,
        java_blocks: String::from_utf8(extract(
            &files.java_blocks.path,
            &files.java_blocks.sha256,
        )?)?,
        block_tags: extract(&files.block_tags.path, &files.block_tags.sha256)?,
        biomes: extract(&files.biomes.path, &files.biomes.sha256)?,
    })
}

/// The archive bytes when `path` holds exactly the pinned archive; `None` when it is
/// missing or differs.
fn read_verified(path: &Path, archive: &Archive) -> Result<Option<Vec<u8>>> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() == archive.size_bytes => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let bytes = std::fs::read(path)?;
    Ok((format!("{:x}", Sha256::digest(&bytes)) == archive.sha256).then_some(bytes))
}

/// Downloads the pinned archive into `cache` after verifying its size and SHA-256.
fn download(archive: &Archive, cache: &Path) -> Result<Vec<u8>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let bytes = runtime.block_on(async {
        let mut response = reqwest::get(&archive.url).await?.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > archive.size_bytes {
                return Err::<_, Box<dyn std::error::Error>>(
                    "download exceeds the pinned archive size".into(),
                );
            }
        }
        Ok(bytes)
    })?;
    if bytes.len() as u64 != archive.size_bytes
        || format!("{:x}", Sha256::digest(&bytes)) != archive.sha256
    {
        return Err(format!("{} does not match the pinned archive", archive.url).into());
    }
    let parent: PathBuf = cache
        .parent()
        .ok_or("archive cache has no parent")?
        .to_owned();
    std::fs::create_dir_all(&parent)?;
    assets::write_blob_atomic(cache, &bytes)?;
    Ok(bytes)
}
