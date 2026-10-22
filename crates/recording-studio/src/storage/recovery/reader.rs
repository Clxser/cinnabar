//! Recovery readers honor durable discard thresholds before admitting backup metadata.

use super::*;
use std::io::Read;

/// A corrupt newest snapshot falls back only to an older admitted backup and explicitly reports that choice.
pub(super) fn latest(path: &Path) -> Result<(Snapshot, String, usize), StorageError> {
    let versions = files::versions(path)?;
    let discarded = lease::discarded_through(path)?;
    let versions = versions
        .into_iter()
        .filter(|(revision, _)| discarded.is_none_or(|discarded| *revision > discarded))
        .collect::<Vec<_>>();
    let count = versions.len();
    let mut issues = Vec::new();
    for (revision, file) in versions.into_iter().take(MAX_RECOVERY_VERSIONS) {
        let result = (|| {
            let mut bytes = Vec::new();
            fs::File::open(&file)
                .map_err(|source| io(&file, source))?
                .take(MAX_RECOVERY_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|source| io(&file, source))?;
            if bytes.len() > MAX_RECOVERY_BYTES {
                return Err(files::error(
                    &file,
                    "Recovery snapshot exceeds its byte budget.",
                ));
            }
            let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(StudioError::from)?;
            if snapshot.version != RECOVERY_VERSION {
                return Err(files::error(
                    &file,
                    "Unsupported recovery format. Open it with the Cinnabar version that created it.",
                ));
            }
            if snapshot.revision != revision {
                return Err(files::error(
                    &file,
                    "Recovery revision does not match its file name.",
                ));
            }
            snapshot.project.validate()?;
            Ok(snapshot)
        })();
        match result {
            Ok(snapshot) => return Ok((snapshot, issues.join("\n"), count)),
            Err(error) => issues.push(format!("Skipped an unreadable newer backup: {error}")),
        }
    }
    Err(files::error(
        path,
        &format!("No valid project recovery was found. {}", issues.join("\n")),
    ))
}
