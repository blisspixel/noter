//! Bounded evidence and exact path representation for uncertain saves.

use std::fmt::{self, Write as _};
use std::path::Path;

use super::save::StorageError;

/// Maximum unresolved save outcomes held by one session.
pub const MAX_SAVE_RECOVERY_RECORDS: usize = 16;
/// Maximum bytes in one retained diagnostic.
pub const MAX_SAVE_RECOVERY_MESSAGE_BYTES: usize = 4 << 10;
/// Maximum encoded bytes in a retained destination path.
pub const MAX_SAVE_RECOVERY_DESTINATION_BYTES: usize = 128 << 10;
/// Maximum bytes in one short destination label.
pub const MAX_SAVE_RECOVERY_LABEL_BYTES: usize = 1 << 10;
/// Guidance retained when a diagnostic exceeds its bound.
pub const SAVE_RECOVERY_TRUNCATION_SUFFIX: &str = "... Recovery detail was shortened to bound memory. Do not save again. Inspect the destination and every retained `.noter-save-*.tmp` sibling before explicit reconciliation.";

struct BoundedTextWriter {
    output: String,
    maximum_bytes: usize,
    truncation_suffix: &'static str,
    truncated: bool,
}

impl BoundedTextWriter {
    fn new(output: String, maximum_bytes: usize, truncation_suffix: &'static str) -> Self {
        debug_assert!(output.capacity() >= maximum_bytes);
        debug_assert!(truncation_suffix.len() <= maximum_bytes);
        Self {
            output,
            maximum_bytes,
            truncation_suffix,
            truncated: false,
        }
    }

    fn finish(mut self) -> String {
        if self.truncated {
            self.output.push_str(self.truncation_suffix);
        }
        debug_assert!(self.output.len() <= self.maximum_bytes);
        self.output
    }
}

impl fmt::Write for BoundedTextWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        if self.output.len().saturating_add(value.len()) <= self.maximum_bytes {
            self.output.push_str(value);
            return Ok(());
        }

        let prefix_limit = self
            .maximum_bytes
            .saturating_sub(self.truncation_suffix.len());
        let mut boundary = self.output.len().min(prefix_limit);
        while !self.output.is_char_boundary(boundary) {
            boundary -= 1;
        }
        self.output.truncate(boundary);
        let available = prefix_limit.saturating_sub(self.output.len());
        let mut boundary = available.min(value.len());
        while !value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        self.output.push_str(&value[..boundary]);
        self.truncated = true;
        Ok(())
    }
}

/// Reserves and builds a short, bounded destination label.
pub fn bounded_destination_label(path: &Path) -> Option<String> {
    let mut output = String::new();
    output
        .try_reserve_exact(MAX_SAVE_RECOVERY_LABEL_BYTES)
        .ok()?;
    let mut writer = BoundedTextWriter::new(output, MAX_SAVE_RECOVERY_LABEL_BYTES, "...");
    match (path.parent().and_then(Path::file_name), path.file_name()) {
        (Some(parent), Some(name)) => {
            let _ = write!(
                writer,
                "{}{}{}",
                parent.to_string_lossy(),
                std::path::MAIN_SEPARATOR,
                name.to_string_lossy()
            );
        }
        (_, Some(name)) => {
            let _ = write!(writer, "{}", name.to_string_lossy());
        }
        _ => {
            let _ = write!(writer, "{}", path.display());
        }
    }
    Some(writer.finish())
}

/// Formats an uncertain outcome into an already-reserved bounded buffer.
pub fn write_save_recovery_message(
    output: String,
    recovery_artifact: &StorageError,
    error: &StorageError,
) -> String {
    let mut writer = BoundedTextWriter::new(
        output,
        MAX_SAVE_RECOVERY_MESSAGE_BYTES,
        SAVE_RECOVERY_TRUNCATION_SUFFIX,
    );
    let _ = write!(
        writer,
        "Save state is uncertain. Noter has stopped every save until you explicitly reconcile this outcome. Recovery follow-up: {recovery_artifact}. Commit detail: {error}"
    );
    writer.finish()
}

/// A reversible representation for copying an operating-system path.
///
/// # Panics
///
/// Panics if allocation fails for the encoded representation. Callers reserve
/// only paths within the recovery destination bound before invoking this.
pub fn recovery_path_clipboard_text(path: &Path) -> String {
    if let Some(path) = path.to_str() {
        return path.to_owned();
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        unix_hex_encoded_path("unix-path-bytes:", path.as_os_str().as_bytes())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        const HEX: &[u8; 16] = b"0123456789abcdef";
        let units = path.as_os_str().encode_wide();
        let unit_count = units.clone().count();
        let mut output = String::new();
        output
            .try_reserve_exact(
                "windows-path-utf16:"
                    .len()
                    .saturating_add(unit_count.saturating_mul(4)),
            )
            .expect("bounded recovery paths fit the clipboard representation");
        output.push_str("windows-path-utf16:");
        for unit in units {
            for shift in [12, 8, 4, 0] {
                output.push(char::from(HEX[usize::from((unit >> shift) & 0x0f)]));
            }
        }
        output
    }
    #[cfg(not(any(unix, windows)))]
    {
        unix_hex_encoded_path(
            "platform-path-encoding:",
            path.as_os_str().as_encoded_bytes(),
        )
    }
}

#[cfg(not(windows))]
fn unix_hex_encoded_path(prefix: &str, bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::new();
    output
        .try_reserve_exact(prefix.len().saturating_add(bytes.len().saturating_mul(2)))
        .expect("bounded recovery paths fit the clipboard representation");
    output.push_str(prefix);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_label_truncates_a_full_parent_at_a_character_boundary() {
        assert_eq!(
            bounded_destination_label(Path::new("note.md")),
            Some("note.md".to_owned())
        );
        let parent = "p".repeat(MAX_SAVE_RECOVERY_LABEL_BYTES - 1);
        let path = Path::new(&parent).join("note.md");
        let label = bounded_destination_label(&path).unwrap();
        assert_eq!(
            label,
            format!("{}...", "p".repeat(MAX_SAVE_RECOVERY_LABEL_BYTES - 3))
        );

        let unicode_parent = "é".repeat(MAX_SAVE_RECOVERY_LABEL_BYTES / 2 - 1);
        let unicode_path = Path::new(&unicode_parent).join("note.md");
        let unicode_label = bounded_destination_label(&unicode_path).unwrap();
        assert!(unicode_label.starts_with(&"é".repeat((MAX_SAVE_RECOVERY_LABEL_BYTES - 3) / 2)));
        assert!(unicode_label.ends_with("..."));
        assert!(unicode_label.len() <= MAX_SAVE_RECOVERY_LABEL_BYTES);

        let prefix = "p".repeat(MAX_SAVE_RECOVERY_LABEL_BYTES - 23);
        let value_path = Path::new(&prefix).join("é".repeat(32));
        let value_label = bounded_destination_label(&value_path).unwrap();
        assert!(value_label.starts_with(&prefix));
        assert!(value_label.ends_with("..."));
        assert!(!value_label.contains('\u{fffd}'));
        assert!(value_label.len() <= MAX_SAVE_RECOVERY_LABEL_BYTES);
    }
}
