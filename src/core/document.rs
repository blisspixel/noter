use ropey::Rope;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::NoterError;

use super::line_endings::LineEndingProfile;
use super::text_format::{Bom, Encoding};

/// The authoritative text and file metadata for one open document.
pub struct Document {
    /// UTF-8 document text.
    pub rope: Rope,
    /// Current save path, or `None` for an untitled document.
    pub path: Option<PathBuf>,
    /// Exact detected line-ending profile and insertion fallback.
    pub line_endings: LineEndingProfile,
    /// Strict on-disk text encoding.
    pub encoding: Encoding,
    /// Optional on-disk UTF-8 byte-order mark.
    pub bom: Bom,
    /// Whether in-memory text differs from the last successful save.
    pub is_dirty: bool,
}

impl Document {
    /// Creates an empty, clean, untitled document using the platform convention.
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            path: None,
            line_endings: LineEndingProfile::detect(""),
            encoding: Encoding::Utf8,
            bom: Bom::Absent,
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
        let (bom, content) = Bom::split_utf8(bytes);
        let text = std::str::from_utf8(content)?;

        Ok(Self {
            rope: Rope::from_str(text),
            path,
            line_endings: LineEndingProfile::detect(text),
            encoding: Encoding::Utf8,
            bom,
            is_dirty: false,
        })
    }

    /// Serializes the current UTF-8 text, including the original BOM when present.
    ///
    /// Existing line endings are emitted exactly as stored in the rope. The editing
    /// model will own insertion and explicit normalization policy.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(self.bom.as_bytes());

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
    use crate::core::line_endings::{LineEnding, LineEndingCounts};
    use tempfile::{NamedTempFile, tempdir};

    #[test]
    fn default_document_matches_new_document() {
        let default = Document::default();
        let new = Document::new();

        assert_eq!(default.rope, new.rope);
        assert_eq!(default.path, new.path);
        assert_eq!(default.line_endings, new.line_endings);
        assert_eq!(default.encoding, new.encoding);
        assert_eq!(default.bom, new.bom);
        assert_eq!(default.is_dirty, new.is_dirty);
    }

    #[test]
    fn loads_strict_utf8_from_path() -> Result<(), NoterError> {
        let mut file = NamedTempFile::new()?;
        file.write_all(b"first\nsecond")?;
        file.flush()?;

        let document = Document::from_path(file.path())?;

        assert_eq!(document.path.as_deref(), Some(file.path()));
        assert_eq!(document.rope.to_string(), "first\nsecond");
        assert_eq!(
            document.line_endings,
            LineEndingProfile::Uniform {
                ending: LineEnding::Lf,
                count: 1
            }
        );
        Ok(())
    }

    #[test]
    fn roundtrip_preserves_bom_and_line_endings() -> Result<(), NoterError> {
        let original = b"\xEF\xBB\xBFhello\r\nworld\r\n";
        let document = Document::from_bytes(original, None)?;

        assert_eq!(
            document.line_endings,
            LineEndingProfile::Uniform {
                ending: LineEnding::CrLf,
                count: 2
            }
        );
        assert_eq!(document.encoding, Encoding::Utf8);
        assert_eq!(document.bom, Bom::Utf8);
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
    fn mixed_line_endings_are_counted_without_normalization() -> Result<(), NoterError> {
        let original = b"one\ntwo\r\nthree\rfour\r\n";
        let document = Document::from_bytes(original, None)?;

        assert_eq!(
            document.line_endings,
            LineEndingProfile::Mixed {
                counts: LineEndingCounts {
                    lf: 1,
                    crlf: 2,
                    cr: 1
                },
                insertion: LineEnding::CrLf
            }
        );
        assert_eq!(document.to_bytes(), original);
        Ok(())
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

    #[test]
    fn failed_replace_removes_temporary_and_preserves_dirty_state() -> Result<(), NoterError> {
        let parent = tempdir()?;
        let destination = parent.path().join("destination");
        std::fs::create_dir(&destination)?;
        let temporary = destination.with_extension(format!("tmp.{}", std::process::id()));
        let mut document = Document::new();
        document.path = Some(destination.clone());
        document.rope = Rope::from_str("must remain dirty");
        document.is_dirty = true;

        let error = document
            .save_atomic()
            .expect_err("a file cannot replace an existing directory");

        assert!(matches!(error, NoterError::AtomicRenameFailed(_)));
        assert!(destination.is_dir());
        assert!(!temporary.exists());
        assert!(document.is_dirty);
        Ok(())
    }
}
