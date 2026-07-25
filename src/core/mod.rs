//! UI-independent document and editing logic.

/// Plain-text document loading, representation, and saving.
pub mod document;
/// Exact line-ending classification and insertion policy.
pub mod line_endings;
/// Monotonic document revision values.
pub mod revision;
/// Revision-tagged, fault-injectable save protocol.
pub mod save;
/// Explicit text encoding and byte-order-mark metadata.
pub mod text_format;
