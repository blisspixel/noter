use thiserror::Error;

use crate::core::save::StorageError;

/// Errors that can occur while loading, editing, or saving a document.
#[derive(Error, Debug)]
pub enum NoterError {
    /// A filesystem operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The document has not yet been assigned a save path.
    #[error("No path associated with document")]
    NoPath,

    /// The source contains bytes that are not valid UTF-8.
    #[error("The file is not valid UTF-8: {0}")]
    InvalidUtf8(#[from] std::str::Utf8Error),

    /// The durable storage adapter could not inspect or prepare the target.
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    /// Save As selected a final entry that cannot be replaced implicitly.
    #[error("Unsupported save target: {0}")]
    UnsupportedTarget(String),

    /// Atomic replacement would separate the selected name from other hard links.
    #[error(
        "The destination has {0} hard links; replacing only this directory entry requires explicit confirmation"
    )]
    HardLinkedTarget(u64),

    /// A document with a path lacks the file observation required for safe Save.
    #[error("The document has no trusted baseline for its save path")]
    MissingFileBaseline,

    /// The monotonic revision counter reached its representable limit.
    #[error("The document revision counter is exhausted")]
    RevisionExhausted,
}
