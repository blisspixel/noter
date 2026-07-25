//! Testable product logic for Noter.
//!
//! GUI framework code stays in the binary crate. Document, editing, persistence,
//! recovery, and application-state behavior belong here so their contracts can be
//! verified without opening a window.

/// UI-independent document and editing logic.
pub mod core;
/// Application error types.
pub mod error;
