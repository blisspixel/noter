//! UI-independent document and editing logic.

/// Plain-text document loading, representation, and saving.
pub mod document;
/// Revision-checked, reversible source transactions.
pub mod edit;
/// Stable destination classification, identity, and content observations.
pub mod file_observation;
/// Production filesystem storage primitives.
pub mod fs_storage;
/// Exact line-ending classification and insertion policy.
pub mod line_endings;
/// Conservative, source-based Markdown diagnostics.
pub mod markdown;
/// Monotonic document revision values.
pub mod revision;
/// Revision-tagged, fault-injectable save protocol.
pub mod save;
/// Explicit text encoding and byte-order-mark metadata.
pub mod text_format;
/// Bounded, revision-aware undo and redo history.
pub mod undo;
