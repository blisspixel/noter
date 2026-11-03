//! Status sentences for a finished save.
//!
//! A save that did not commit names the folder and the next step. The storage
//! engine keeps its own diagnostic, including the operation name and the
//! operating-system code, for recovery records. A save that synced the file
//! and its folder, and that has no warning, can show a short receipt. Every
//! other committed save keeps its existing follow-up.

use std::path::Path;
use std::time::{Duration, Instant};

use noter::core::save::{Durability, SaveWarnings, StorageError};
use noter::core::terminal_text::{escaped_cli_path, is_terminal_unsafe};

/// How long the quiet receipt replaces `Saved`.
///
/// The terminal status line uses the same interval.
pub const SAVE_RECEIPT_LIFETIME: Duration = Duration::from_secs(3);

/// A locale short time longer than this is not shown. Real short times fit.
const MAX_RECEIPT_TIME_CHARS: usize = 64;

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

/// Receipt kept after a fully durable save. It is absent for every other outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveReceipt {
    text: String,
    shown_at: Instant,
}

impl SaveReceipt {
    /// Stores `text` as shown at `shown_at`.
    pub const fn new(text: String, shown_at: Instant) -> Self {
        Self { text, shown_at }
    }

    /// Returns the receipt sentence.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Time left before the cell returns to `Saved`.
    ///
    /// `None` means the receipt has expired, including the instant it reaches
    /// its lifetime. A clock that moved backwards still reports the full lifetime.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.shown_at);
        match SAVE_RECEIPT_LIFETIME.checked_sub(elapsed) {
            Some(remaining) if !remaining.is_zero() => Some(remaining),
            _ => None,
        }
    }
}

/// Sentence for a file-and-directory sync with no warning.
///
/// `local_time` is the locale short time captured once at the save. File-only
/// sync, best-effort sync, any warning, an empty time, a time that contains a
/// terminal control, and a time longer than a short clock reading all return
/// `None`. A control is rejected before trimming, so a trailing newline does
/// not become a clock reading. Callers then keep the ordinary save status.
#[must_use]
pub fn durable_receipt_text(
    durability: Durability,
    warnings: &SaveWarnings,
    byte_count: u64,
    local_time: &str,
) -> Option<String> {
    if durability != Durability::FileAndDirectorySynced || !warnings.is_empty() {
        return None;
    }
    // Trim only after the control check. Newline is whitespace, and trimming
    // it first would hide a character the terminal must not receive.
    if local_time.chars().any(is_terminal_unsafe) {
        return None;
    }
    let local_time = local_time.trim();
    if local_time.is_empty() || local_time.chars().count() > MAX_RECEIPT_TIME_CHARS {
        return None;
    }
    Some(format!("Saved {byte_count} B, synced to disk {local_time}"))
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
    use std::time::Instant;

    use noter::core::save::{
        Durability, FileIdentity, RetainedSaveSibling, SaveStage, SaveWarnings, StorageError,
    };

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

    #[test]
    fn only_a_clean_file_and_directory_sync_names_the_disk() {
        let clear = SaveWarnings::default();
        let receipt =
            durable_receipt_text(Durability::FileAndDirectorySynced, &clear, 664, "14:02")
                .expect("receipt");
        assert_eq!(receipt, "Saved 664 B, synced to disk 14:02");
        assert_eq!(
            durable_receipt_text(Durability::FileAndDirectorySynced, &clear, 4, " 午後2:02 ",)
                .as_deref(),
            Some("Saved 4 B, synced to disk 午後2:02")
        );

        for durability in [Durability::FileSynced, Durability::BestEffort] {
            assert!(
                durable_receipt_text(durability, &clear, 664, "14:02").is_none(),
                "{durability:?}"
            );
        }
        let cleanup = SaveWarnings::new(
            vec![StorageError::new(SaveStage::Cleanup, "kept a sibling")],
            Vec::new(),
        );
        let durability = SaveWarnings::new(
            Vec::new(),
            vec![StorageError::new(
                SaveStage::SyncParent,
                "directory sync failed",
            )],
        );
        let sibling = SaveWarnings::new(Vec::new(), Vec::new()).with_retained(vec![
            RetainedSaveSibling::new(
                PathBuf::from(".noter-save-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.tmp"),
                FileIdentity::new(1, 2),
            ),
        ]);
        for warnings in [&cleanup, &durability, &sibling] {
            assert!(
                durable_receipt_text(Durability::FileAndDirectorySynced, warnings, 664, "14:02")
                    .is_none()
            );
        }
        let exact = "1".repeat(MAX_RECEIPT_TIME_CHARS);
        assert!(
            durable_receipt_text(Durability::FileAndDirectorySynced, &clear, 1, &exact).is_some(),
            "a {MAX_RECEIPT_TIME_CHARS} character time still fits"
        );
        let long = "1".repeat(MAX_RECEIPT_TIME_CHARS + 1);
        for time in [
            "",
            " ",
            "14:02\n",
            "14\u{1b}02",
            "\u{202e}14:02",
            "\u{2028}14:02",
            long.as_str(),
        ] {
            assert!(
                durable_receipt_text(Durability::FileAndDirectorySynced, &clear, 1, time).is_none(),
                "{time:?}"
            );
        }
    }

    #[test]
    fn the_receipt_hides_when_its_lifetime_ends() {
        let shown = Instant::now();
        let receipt = SaveReceipt::new("Saved 1 B, synced to disk 14:02".to_owned(), shown);
        assert_eq!(receipt.text(), "Saved 1 B, synced to disk 14:02");
        assert_eq!(receipt.remaining(shown), Some(SAVE_RECEIPT_LIFETIME));
        let just_before = (shown + SAVE_RECEIPT_LIFETIME)
            .checked_sub(Duration::from_nanos(1))
            .expect("one nanosecond before expiry");
        assert!(receipt.remaining(just_before).is_some());
        assert!(receipt.remaining(shown + SAVE_RECEIPT_LIFETIME).is_none());
        let earlier = shown
            .checked_sub(Duration::from_secs(1))
            .expect("one second before the receipt");
        assert_eq!(receipt.remaining(earlier), Some(SAVE_RECEIPT_LIFETIME));
    }
}
