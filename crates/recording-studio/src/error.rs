//! Errors shared by project admission and atomic editing.

#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{field} exceeds its limit ({actual} > {limit})")]
    Limit {
        field: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error(
        "unsupported recording project version {0}; update Cinnabar or open a compatible backup"
    )]
    Version(u32),
    #[error("{kind} {id} was not found")]
    Missing { kind: &'static str, id: u64 },
    #[error("project identifiers are exhausted")]
    IdentifiersExhausted,
    #[error("media time is outside the supported range")]
    TimeOverflow,
    #[error("project cannot be decoded: {0}")]
    Decode(#[from] serde_json::Error),
}

/// Rejects a collection before an editor or decoder can traverse unbounded work.
pub(crate) fn limit(field: &'static str, actual: usize, maximum: usize) -> Result<(), StudioError> {
    if actual > maximum {
        return Err(StudioError::Limit {
            field,
            actual,
            limit: maximum,
        });
    }
    Ok(())
}
