use thiserror::Error;

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

    /// Replacing the destination with the fully written temporary file failed.
    #[error("Failed to rename temporary file: {0}")]
    AtomicRenameFailed(String),
}
