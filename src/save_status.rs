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

/// Notice for a save that committed and still has cleanup or durability
/// follow-up. It says the save happened. It is not a failed save.
pub fn committed_save_notice(durability_at_risk: bool, details: &[&str]) -> String {
    let detail = details
        .iter()
        .copied()
        .filter(|detail| !detail.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if durability_at_risk {
        let mut notice = String::from(
            "Keep this window open and Save again. Saved, but storage durability is uncertain.",
        );
        if !detail.is_empty() {
            notice.push(' ');
            notice.push_str(&detail);
        }
        notice
    } else if detail.is_empty() {
        "Saved. Follow-up is required.".to_owned()
    } else {
        format!("Saved. Follow-up: {detail}")
    }
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
    fn a_committed_save_notice_leads_with_the_next_step_and_is_not_an_error() {
        let cleanup = "A displaced recovery artifact was preserved as `.noter-save-a123.tmp` beside the destination. Inspect its contents before removal.";
        let notice = committed_save_notice(false, &[cleanup]);

        assert!(notice.starts_with("Saved. Follow-up:"), "{notice}");
        assert!(notice.contains(cleanup), "{notice}");
        assert!(!notice.contains("Error:"), "{notice}");

        let durability = committed_save_notice(true, &["directory sync failed"]);
        assert!(
            durability.starts_with("Keep this window open and Save again."),
            "{durability}"
        );
        let step = durability.find("Keep this window open").expect("step");
        let reason = durability.find("directory sync failed").expect("reason");
        assert!(step < reason, "{durability}");
        assert!(!durability.contains("Error:"), "{durability}");
        assert_eq!(
            committed_save_notice(true, &["", ""]),
            "Keep this window open and Save again. Saved, but storage durability is uncertain."
        );
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
