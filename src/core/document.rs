use ropey::Rope;
use std::path::{Path, PathBuf};

use crate::error::NoterError;

use super::file_observation::{inspect_target, read_regular_file};
use super::fs_storage::FilesystemStorage;
use super::line_endings::LineEndingProfile;
use super::revision::Revision;
use super::save::{
    SaveOutcome, SaveSnapshot, SaveStage, TargetExpectation, TargetState, save_snapshot,
};
use super::text_format::{Bom, Encoding};

/// The authoritative text and file metadata for one open document.
pub struct Document {
    /// UTF-8 document text.
    rope: Rope,
    /// Current save path, or `None` for an untitled document.
    path: Option<PathBuf>,
    /// Exact detected line-ending profile and insertion fallback.
    line_endings: LineEndingProfile,
    /// Strict on-disk text encoding.
    encoding: Encoding,
    /// Optional on-disk UTF-8 byte-order mark.
    bom: Bom,
    revision: Revision,
    saved_revision: Revision,
    saved_target: Option<TargetExpectation>,
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
            revision: Revision::INITIAL,
            saved_revision: Revision::INITIAL,
            saved_target: None,
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
        let (bytes, observation) = read_regular_file(path, SaveStage::InspectInitial)?;
        let mut document = Self::from_bytes(&bytes)?;
        document.path = Some(path.to_path_buf());
        document.saved_target = Some(TargetExpectation::Existing(observation));
        Ok(document)
    }

    /// Builds an untitled document from strict UTF-8 bytes.
    ///
    /// A UTF-8 byte-order mark is recorded but excluded from the rope. Existing
    /// line endings remain in the rope so an unedited document round-trips exactly.
    ///
    /// # Errors
    ///
    /// Returns [`NoterError::InvalidUtf8`] when the bytes after an optional UTF-8
    /// byte-order mark are not valid UTF-8.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NoterError> {
        let (bom, content) = Bom::split_utf8(bytes);
        let text = std::str::from_utf8(content)?;

        Ok(Self {
            rope: Rope::from_str(text),
            path: None,
            line_endings: LineEndingProfile::detect(text),
            encoding: Encoding::Utf8,
            bom,
            revision: Revision::INITIAL,
            saved_revision: Revision::INITIAL,
            saved_target: None,
        })
    }

    /// Returns the authoritative UTF-8 text buffer.
    pub const fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Returns the current save path, or `None` for an untitled document.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Returns the exact detected line-ending profile and insertion fallback.
    pub const fn line_endings(&self) -> &LineEndingProfile {
        &self.line_endings
    }

    /// Returns the strict on-disk text encoding.
    pub const fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Returns the optional on-disk UTF-8 byte-order mark.
    pub const fn bom(&self) -> Bom {
        self.bom
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

    /// Returns the current monotonic content revision.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns whether the current revision differs from the last committed revision.
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Replaces authoritative text and advances the revision exactly once when changed.
    ///
    /// # Errors
    ///
    /// Returns [`NoterError::RevisionExhausted`] if the monotonic counter cannot
    /// advance without wrapping.
    pub fn replace_text(&mut self, text: &str) -> Result<bool, NoterError> {
        let replacement = Rope::from_str(text);
        if replacement == self.rope {
            return Ok(false);
        }
        self.revision = self
            .revision
            .checked_next()
            .ok_or(NoterError::RevisionExhausted)?;
        self.rope = replacement;
        self.line_endings = LineEndingProfile::detect(text);
        Ok(true)
    }

    /// Saves the current revision to its trusted existing path.
    ///
    /// Dirty state clears only when the exact current revision commits. Conflict,
    /// proven failure, and unknown commit state remain explicit outcomes.
    ///
    /// # Errors
    ///
    /// Returns [`NoterError::NoPath`] for an untitled document or
    /// [`NoterError::MissingFileBaseline`] if no trusted load/save observation
    /// exists for the current path.
    pub fn save(&mut self) -> Result<SaveOutcome, NoterError> {
        self.save_with_hard_link_policy(false)
    }

    /// Saves after explicit confirmation that only one hard-linked name changes.
    ///
    /// Use this only after the user has acknowledged that other hard links will
    /// continue to reference the previous file object.
    ///
    /// # Errors
    ///
    /// Returns the same path and baseline errors as [`Self::save`].
    pub fn save_confirming_hard_link_replacement(&mut self) -> Result<SaveOutcome, NoterError> {
        self.save_with_hard_link_policy(true)
    }

    fn save_with_hard_link_policy(
        &mut self,
        hard_link_confirmed: bool,
    ) -> Result<SaveOutcome, NoterError> {
        let path = self.path.clone().ok_or(NoterError::NoPath)?;
        let expected = self.saved_target.ok_or(NoterError::MissingFileBaseline)?;
        require_hard_link_confirmation(expected, hard_link_confirmed)?;
        Ok(self.save_to_expectation(path, expected, false))
    }

    /// Saves the current revision to a user-selected path.
    ///
    /// Existing regular files are versioned at selection time and revalidated by
    /// the save protocol. Final links, directories, and other special entries are
    /// refused. The document adopts the path only after the current revision commits.
    ///
    /// # Errors
    ///
    /// Returns a storage error if the selected path cannot be inspected safely or
    /// [`NoterError::UnsupportedTarget`] for a special final entry.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<SaveOutcome, NoterError> {
        self.save_as_with_hard_link_policy(path.as_ref(), false)
    }

    /// Saves to a selected path after explicit hard-link replacement confirmation.
    ///
    /// # Errors
    ///
    /// Returns the same target-inspection errors as [`Self::save_as`].
    pub fn save_as_confirming_hard_link_replacement(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<SaveOutcome, NoterError> {
        self.save_as_with_hard_link_policy(path.as_ref(), true)
    }

    fn save_as_with_hard_link_policy(
        &mut self,
        path: &Path,
        hard_link_confirmed: bool,
    ) -> Result<SaveOutcome, NoterError> {
        if self.path.as_deref() == Some(path) {
            return self.save_with_hard_link_policy(hard_link_confirmed);
        }
        let expected = match inspect_target(path, SaveStage::InspectInitial)? {
            TargetState::Missing => TargetExpectation::Missing,
            TargetState::Regular(observation) => TargetExpectation::Existing(observation),
            TargetState::Special(kind) => {
                return Err(NoterError::UnsupportedTarget(format!("{kind:?}")));
            }
        };
        require_hard_link_confirmation(expected, hard_link_confirmed)?;
        Ok(self.save_to_expectation(path.to_path_buf(), expected, true))
    }

    fn save_to_expectation(
        &mut self,
        path: PathBuf,
        expected: TargetExpectation,
        adopt_path: bool,
    ) -> SaveOutcome {
        let snapshot = SaveSnapshot::new(self.revision, path.clone(), expected, self.to_bytes());
        let outcome = save_snapshot(&mut FilesystemStorage, &snapshot);
        if let SaveOutcome::Committed {
            revision,
            observation,
            ..
        } = &outcome
            && *revision == self.revision
        {
            self.saved_revision = *revision;
            self.saved_target = Some(TargetExpectation::Existing(*observation));
            if adopt_path {
                self.path = Some(path);
            }
        }
        outcome
    }
}

const fn require_hard_link_confirmation(
    expected: TargetExpectation,
    confirmed: bool,
) -> Result<(), NoterError> {
    if let TargetExpectation::Existing(observation) = expected
        && observation.link_count() > 1
        && !confirmed
    {
        return Err(NoterError::HardLinkedTarget(observation.link_count()));
    }
    Ok(())
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::*;
    use crate::core::line_endings::{LineEnding, LineEndingCounts};
    use tempfile::{NamedTempFile, tempdir};

    #[test]
    fn default_document_matches_new_document() {
        let default = Document::default();
        let new = Document::new();

        assert_eq!(default.rope(), new.rope());
        assert_eq!(default.path(), new.path());
        assert_eq!(default.line_endings(), new.line_endings());
        assert_eq!(default.encoding(), new.encoding());
        assert_eq!(default.bom(), new.bom());
        assert_eq!(default.revision(), new.revision());
        assert_eq!(default.is_dirty(), new.is_dirty());
    }

    #[test]
    fn loads_strict_utf8_from_path() -> Result<(), NoterError> {
        let mut file = NamedTempFile::new()?;
        file.write_all(b"first\nsecond")?;
        file.flush()?;

        let document = Document::from_path(file.path())?;

        assert_eq!(document.path(), Some(file.path()));
        assert_eq!(document.rope().to_string(), "first\nsecond");
        assert_eq!(
            document.line_endings(),
            &LineEndingProfile::Uniform {
                ending: LineEnding::Lf,
                count: 1
            }
        );
        Ok(())
    }

    #[test]
    fn roundtrip_preserves_bom_and_line_endings() -> Result<(), NoterError> {
        let original = b"\xEF\xBB\xBFhello\r\nworld\r\n";
        let document = Document::from_bytes(original)?;

        assert_eq!(
            document.line_endings(),
            &LineEndingProfile::Uniform {
                ending: LineEnding::CrLf,
                count: 2
            }
        );
        assert_eq!(document.encoding(), Encoding::Utf8);
        assert_eq!(document.bom(), Bom::Utf8);
        assert_eq!(document.path(), None);
        assert_eq!(original.as_slice(), document.to_bytes());
        Ok(())
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let error = Document::from_bytes(b"valid prefix\xFF")
            .err()
            .expect("invalid UTF-8 must produce an error");

        assert!(matches!(error, NoterError::InvalidUtf8(_)));
    }

    #[test]
    fn mixed_line_endings_are_counted_without_normalization() -> Result<(), NoterError> {
        let original = b"one\ntwo\r\nthree\rfour\r\n";
        let document = Document::from_bytes(original)?;

        assert_eq!(
            document.line_endings(),
            &LineEndingProfile::Mixed {
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
        document
            .replace_text("unsaved")
            .expect("the initial edit should advance the revision");

        let error = document
            .save()
            .expect_err("an untitled document must require Save As");

        assert!(matches!(error, NoterError::NoPath));
        assert!(document.is_dirty());
    }

    #[test]
    fn durable_save_replaces_complete_file() -> Result<(), NoterError> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"original")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("Hello durable save!")?;

        let outcome = document.save()?;

        assert!(matches!(outcome, SaveOutcome::Committed { .. }));
        assert!(!document.is_dirty());
        assert_eq!(fs::read(&path)?, document.to_bytes());
        Ok(())
    }

    #[test]
    fn save_as_refuses_directory_and_preserves_dirty_state() -> Result<(), NoterError> {
        let parent = tempdir()?;
        let destination = parent.path().join("destination");
        fs::create_dir(&destination)?;
        let mut document = Document::new();
        document.replace_text("must remain dirty")?;

        let error = document
            .save_as(&destination)
            .expect_err("Save As must refuse an existing directory");

        assert!(matches!(error, NoterError::UnsupportedTarget(_)));
        assert!(destination.is_dir());
        assert!(document.is_dirty());
        assert_eq!(document.path, None);
        Ok(())
    }

    #[test]
    fn external_change_becomes_conflict_without_overwrite() -> Result<(), NoterError> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"loaded version")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("my edit")?;
        fs::write(&path, b"external edit")?;

        let outcome = document.save()?;

        assert!(matches!(outcome, SaveOutcome::Conflict { .. }));
        assert_eq!(fs::read(&path)?, b"external edit");
        assert!(document.is_dirty());
        Ok(())
    }

    #[test]
    fn successful_save_as_adopts_path_only_after_commit() -> Result<(), NoterError> {
        let directory = tempdir()?;
        let path = directory.path().join("new-note.txt");
        let mut document = Document::new();
        document.replace_text("new document")?;

        let outcome = document.save_as(&path)?;

        assert!(matches!(outcome, SaveOutcome::Committed { .. }));
        assert_eq!(document.path.as_deref(), Some(path.as_path()));
        assert!(!document.is_dirty());
        assert_eq!(fs::read(path)?, b"new document");
        Ok(())
    }

    #[test]
    fn hard_link_replacement_requires_and_honors_explicit_confirmation() -> Result<(), NoterError> {
        let directory = tempdir()?;
        let selected = directory.path().join("selected.txt");
        let other_link = directory.path().join("other-link.txt");
        fs::write(&selected, b"shared original")?;
        fs::hard_link(&selected, &other_link)?;
        let mut document = Document::from_path(&selected)?;
        document.replace_text("selected replacement")?;

        let refusal = document
            .save()
            .expect_err("ordinary Save must not split hard links silently");

        assert!(matches!(refusal, NoterError::HardLinkedTarget(count) if count >= 2));
        assert_eq!(fs::read(&selected)?, b"shared original");
        assert_eq!(fs::read(&other_link)?, b"shared original");
        assert!(document.is_dirty());

        let outcome = document.save_confirming_hard_link_replacement()?;

        assert!(matches!(outcome, SaveOutcome::Committed { .. }));
        assert_eq!(fs::read(&selected)?, b"selected replacement");
        assert_eq!(fs::read(&other_link)?, b"shared original");
        assert!(!document.is_dirty());
        Ok(())
    }
}
