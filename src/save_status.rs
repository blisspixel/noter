//! Plain status for a save that proved it did not commit.
//!
//! The storage engine keeps its own diagnostic, including the operation name
//! and the operating-system code, for recovery records. A person saving a
//! document needs the folder, the fact that the open text was not written,
//! and a next step.

use std::path::Path;

use noter::core::save::StorageError;
use noter::core::terminal_text::escaped_cli_path;

/// The status for [`noter::core::save::SaveOutcome::NotCommitted`].
///
/// `destination` is the path the user asked to write. The folder in the
/// sentence is its parent. A cleanup failure is appended so a private
/// artifact that could not be removed stays visible.
pub fn not_committed_status(destination: &Path, cleanup_error: Option<&StorageError>) -> String {
    let mut message = format!(
        "Not saved: could not write in {}. This text is still open, and the file on disk was not changed. Choose another location.",
        folder_label(destination)
    );
    append_cleanup_warning(&mut message, cleanup_error);
    message
}

/// Appends the retained-artifact warning used when cleanup fails after a
/// save that did not commit, or after a conflict.
pub fn append_cleanup_warning(message: &mut String, cleanup_error: Option<&StorageError>) {
    if let Some(cleanup_error) = cleanup_error {
        message.push_str(" Cleanup also failed and a private artifact may remain: ");
        message.push_str(&cleanup_error.to_string());
    }
}

fn folder_label(destination: &Path) -> String {
    match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            format!("`{}`", escaped_cli_path(parent))
        }
        _ => "the current directory".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use noter::core::save::{SaveStage, StorageError};

    use super::*;

    #[test]
    fn a_failed_save_names_the_folder_and_hides_the_engine_diagnostic() {
        let destination = Path::new("readonly").join("note.txt");
        let status = not_committed_status(&destination, None);

        assert_eq!(
            status,
            "Not saved: could not write in `readonly`. This text is still open, and the file on disk was not changed. Choose another location."
        );
        assert!(!status.contains("PermissionDenied"));
        assert!(!status.contains("OS code"));
        assert!(!status.contains("create private sibling"));
        assert!(!status.contains('\n'));
    }

    #[test]
    fn a_bare_file_name_names_the_current_directory() {
        let status = not_committed_status(Path::new("note.txt"), None);

        assert!(status.contains("could not write in the current directory"));
        assert!(!status.contains("PermissionDenied"));
    }

    #[test]
    fn a_control_character_in_the_folder_is_escaped() {
        let destination = PathBuf::from("read\nonly").join("note.txt");
        let status = not_committed_status(&destination, None);

        assert!(status.contains("read\\nonly"));
        assert!(!status.contains('\n'));
    }

    #[test]
    fn a_cleanup_failure_stays_visible_after_the_plain_sentence() {
        let cleanup = StorageError::new(SaveStage::Cleanup, "private artifact was preserved");
        let status = not_committed_status(&Path::new("readonly").join("note.txt"), Some(&cleanup));

        assert!(status.contains("could not write in `readonly`"));
        assert!(status.contains("Cleanup also failed and a private artifact may remain:"));
        assert!(status.contains("private artifact was preserved"));
        assert!(!status.contains("OS code"));
    }
}
