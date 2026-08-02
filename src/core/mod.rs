//! UI-independent document and editing logic.

/// Pure external-change classification and conflict decisions.
pub mod conflict;
/// Plain-text document loading, representation, and saving.
pub mod document;
/// Revision-checked, reversible source transactions.
pub mod edit;
/// Stable destination classification, identity, and content observations.
pub mod file_observation;
/// Production filesystem storage primitives.
pub mod fs_storage;
/// Pure destructive-action lifecycle state machine.
pub mod lifecycle;
/// Shared product resource ceilings.
pub mod limits;
/// Exact line-ending classification and insertion policy.
pub mod line_endings;
/// Conservative, source-based Markdown diagnostics.
pub mod markdown;
/// Allocation-free logical-line navigation.
pub mod navigation;
/// Monotonic document revision values.
pub mod revision;
/// Revision-tagged, fault-injectable save protocol.
pub mod save;
/// Bounded literal search and replacement policy.
pub mod search;
/// Explicit text encoding and byte-order-mark metadata.
pub mod text_format;
/// Bounded, revision-aware undo and redo history.
pub mod undo;
