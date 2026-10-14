//! Canonical profile-bound evidence records. No retrieval or runtime authority.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod lifecycle;
pub mod reader;
pub mod record;
pub mod render;
pub mod scope;

pub use lifecycle::*;
pub use record::*;
pub use scope::*;

/// Validation, canonical encoding, or authority error.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// A bounded field does not meet the canonical format.
    #[error("invalid memory field: {0}")]
    Invalid(&'static str),
    /// A record or source belongs to a different profile or exact place.
    #[error("memory scope mismatch")]
    ScopeMismatch,
    /// The persisted canonical bytes do not match their digest.
    #[error("memory digest mismatch")]
    DigestMismatch,
    /// A native caller did not have the required owner authority.
    #[error("memory operation is not authorized")]
    Unauthorized,
    /// Serialization failed without altering canonical state.
    #[error("memory encoding failed: {0}")]
    Encoding(#[from] serde_json::Error),
}

pub(crate) fn bounded(value: &str, max: usize, field: &'static str) -> Result<(), MemoryError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(MemoryError::Invalid(field));
    }
    Ok(())
}
