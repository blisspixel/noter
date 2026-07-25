use ropey::Rope;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::NoterError;

/// The byte sequence used to terminate logical lines in a text file.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum LineEnding {
    /// Unix line feed (`\n`).
    Lf,
    /// Windows carriage return plus line feed (`\r\n`).
    CrLf,
    /// Legacy carriage return (`\r`).
    Cr,
}

impl LineEnding {
    /// Returns the encoded line-ending sequence.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }

    /// Returns the first detected line ending and the UTF-8 BOM length.
    pub fn detect_from_bytes(bytes: &[u8]) -> (Self, usize) {
        let bom_len = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        };

        let content = &bytes[bom_len..];
        for (index, byte) in content.iter().enumerate() {
            match byte {
                b'\r' if content.get(index + 1) == Some(&b'\n') => {
                    return (Self::CrLf, bom_len);
                }
                b'\r' => return (Self::Cr, bom_len),
                b'\n' => return (Self::Lf, bom_len),
                _ => {}
            }
        }

        // A file without a line break has no on-disk convention to preserve.
        #[cfg(windows)]
        return (Self::CrLf, bom_len);
        #[cfg(not(windows))]
        return (Self::Lf, bom_len);
    }
}

/// The authoritative text and file metadata for one open document.
pub struct Document {
    /// UTF-8 document text.
    pub rope: Rope,
    /// Current save path, or `None` for an untitled document.
    pub path: Option<PathBuf>,
    /// Detected or user-selected line-ending convention.
    pub line_ending: LineEnding,
    /// Whether the loaded file began with a UTF-8 byte-order mark.
    pub had_bom: bool,
    /// Whether in-memory text differs from the last successful save.
    pub is_dirty: bool,
}

impl Document {
    /// Creates an empty, clean, untitled document using the platform convention.
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            path: None,
            #[cfg(windows)]
            line_ending: LineEnding::CrLf,
            #[cfg(not(windows))]
            line_ending: LineEnding::Lf,
            had_bom: false,
            is_dirty: false,
        }
    }

    /// Loads a strict UTF-8 document from disk.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file cannot be read, or
    /// [`NoterError::InvalidUtf8`] when its content is not valid UTF-8.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, NoterError> {
        let path = path.as_ref();
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes, Some(path.to_path_buf()))
    }

    /// Builds a document from strict UTF-8 bytes and an optional source path.
    ///
    /// A UTF-8 byte-order mark is recorded but excluded from the rope. Existing
    /// line endings remain in the rope so an unedited document round-trips exactly.
    ///
    /// # Errors
    ///
    /// Returns [`NoterError::InvalidUtf8`] when the bytes after an optional UTF-8
    /// byte-order mark are not valid UTF-8.
    pub fn from_bytes(bytes: &[u8], path: Option<PathBuf>) -> Result<Self, NoterError> {
        let (line_ending, bom_len) = LineEnding::detect_from_bytes(bytes);
        let had_bom = bom_len > 0;
        let text = std::str::from_utf8(&bytes[bom_len..])?;

        Ok(Self {
            rope: Rope::from_str(text),
            path,
            line_ending,
            had_bom,
            is_dirty: false,
        })
    }

    /// Serializes the current UTF-8 text, including the original BOM when present.
    ///
    /// Existing line endings are emitted exactly as stored in the rope. The editing
    /// model will own insertion and explicit normalization policy.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        if self.had_bom {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }

        for chunk in self.rope.chunks() {
            bytes.extend_from_slice(chunk.as_bytes());
        }

        bytes
    }

    /// Writes the document to a sibling temporary file and replaces its save path.
    ///
    /// The dirty flag is cleared only after the replacement succeeds. This is an
    /// interim implementation of safety property S1. Platform durability and metadata
    /// preservation are tracked as the next trust-kernel milestone.
    ///
    /// # Errors
    ///
    /// Returns [`NoterError::NoPath`] for an untitled document, an I/O error while
    /// writing or syncing, or [`NoterError::AtomicRenameFailed`] if replacement fails.
    pub fn save_atomic(&mut self) -> Result<(), NoterError> {
        let path = self.path.as_ref().ok_or(NoterError::NoPath)?;
        let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));

        {
            let mut file = File::create(&tmp_path)?;
            file.write_all(&self.to_bytes())?;
            file.sync_all()?;
        }

        if let Err(error) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(NoterError::AtomicRenameFailed(error.to_string()));
        }

        self.is_dirty = false;
        Ok(())
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn detects_line_endings_and_bom() {
        assert_eq!(
            LineEnding::detect_from_bytes(b"hello\nworld"),
            (LineEnding::Lf, 0)
        );
        assert_eq!(
            LineEnding::detect_from_bytes(b"hello\r\nworld"),
            (LineEnding::CrLf, 0)
        );
        assert_eq!(
            LineEnding::detect_from_bytes(b"\xEF\xBB\xBFhello\rworld"),
            (LineEnding::Cr, 3)
        );
    }

    #[test]
    fn line_endings_encode_expected_sequences() {
        for (line_ending, expected) in [
            (LineEnding::Lf, "\n"),
            (LineEnding::CrLf, "\r\n"),
            (LineEnding::Cr, "\r"),
        ] {
            assert_eq!(line_ending.as_str(), expected);
        }
    }

    #[test]
    fn loads_strict_utf8_from_path() -> Result<(), NoterError> {
        let mut file = NamedTempFile::new()?;
        file.write_all(b"first\nsecond")?;
        file.flush()?;

        let document = Document::from_path(file.path())?;

        assert_eq!(document.path.as_deref(), Some(file.path()));
        assert_eq!(document.rope.to_string(), "first\nsecond");
        assert_eq!(document.line_ending, LineEnding::Lf);
        Ok(())
    }

    #[test]
    fn roundtrip_preserves_bom_and_line_endings() -> Result<(), NoterError> {
        let original = b"\xEF\xBB\xBFhello\r\nworld\r\n";
        let document = Document::from_bytes(original, None)?;

        assert_eq!(document.line_ending, LineEnding::CrLf);
        assert!(document.had_bom);
        assert_eq!(original.as_slice(), document.to_bytes());
        Ok(())
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let error = Document::from_bytes(b"valid prefix\xFF", None)
            .err()
            .expect("invalid UTF-8 must produce an error");

        assert!(matches!(error, NoterError::InvalidUtf8(_)));
    }

    #[test]
    fn saving_an_untitled_document_preserves_dirty_state() {
        let mut document = Document::new();
        document.rope = Rope::from_str("unsaved");
        document.is_dirty = true;

        let error = document
            .save_atomic()
            .expect_err("an untitled document must require Save As");

        assert!(matches!(error, NoterError::NoPath));
        assert!(document.is_dirty);
    }

    #[test]
    fn atomic_save_replaces_complete_file() -> Result<(), NoterError> {
        let file = NamedTempFile::new()?;
        let path = file.path().to_path_buf();
        let mut document = Document::new();
        document.path = Some(path.clone());
        document.rope = Rope::from_str("Hello Atomic!");
        document.is_dirty = true;

        document.save_atomic()?;

        let mut saved = Vec::new();
        File::open(&path)?.read_to_end(&mut saved)?;
        assert!(!document.is_dirty);
        assert_eq!(saved, document.to_bytes());
        Ok(())
    }
}
