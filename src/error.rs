use thiserror::Error;

#[derive(Error, Debug)]
pub enum NoterError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("No path associated with document")]
    NoPath,

    #[error("Failed to rename temporary file: {0}")]
    AtomicRenameFailed(String),
}
