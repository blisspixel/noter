use ropey::Rope;
use std::path::{Path, PathBuf};

use crate::error::NoterError;

use super::edit::{
    AppliedTransaction, EditError, EditOrigin, EditTimestamp, EditTransaction, Selection,
};
use super::file_observation::{inspect_target, read_regular_file};
use super::fs_storage::FilesystemStorage;
use super::limits::MAX_DOCUMENT_BYTES;
use super::line_endings::LineEndingProfile;
use super::revision::Revision;
use super::save::{
    ContentFingerprint, SaveOutcome, SaveSnapshot, SaveStage, TargetExpectation, TargetState,
    save_snapshot,
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
    content_fingerprint: ContentFingerprint,
    saved_content_fingerprint: ContentFingerprint,
    saved_target: Option<TargetExpectation>,
}

/// A Save As selection paired with the exact target state observed when the
/// user chose it.
///
/// The fields are private so confirmation cannot silently replace the
/// expectation with a later observation. Reusing a stale preparation is safe:
/// the save protocol reports a conflict if the selected entry changed.
#[derive(Clone, Debug)]
pub struct PreparedSaveAs {
    path: PathBuf,
    expected: TargetExpectation,
    adopt_path: bool,
}

impl PreparedSaveAs {
    /// Returns the observed hard-link count when the selected entry existed.
    #[must_use]
    pub const fn hard_link_count(&self) -> Option<u64> {
        match self.expected {
            TargetExpectation::Existing(observation) => Some(observation.link_count()),
            TargetExpectation::Missing => None,
        }
    }
}

impl Document {
    /// Creates an empty, clean, untitled document using the platform convention.
    pub fn new() -> Self {
        let fingerprint = ContentFingerprint::from_bytes(b"");
        Self {
            rope: Rope::new(),
            path: None,
            line_endings: LineEndingProfile::detect(""),
            encoding: Encoding::Utf8,
            bom: Bom::Absent,
            revision: Revision::INITIAL,
            content_fingerprint: fingerprint,
            saved_content_fingerprint: fingerprint,
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
    /// Returns [`NoterError::DocumentTooLarge`] when serialized input exceeds
    /// the shared document ceiling, or [`NoterError::InvalidUtf8`] when bytes
    /// after an optional UTF-8 byte-order mark are not valid UTF-8.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NoterError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(NoterError::DocumentTooLarge {
                actual: bytes.len(),
                maximum: MAX_DOCUMENT_BYTES,
            });
        }
        let (bom, content) = Bom::split_utf8(bytes);
        let text = std::str::from_utf8(content)?;
        let fingerprint = ContentFingerprint::from_bytes(bytes);

        Ok(Self {
            rope: Rope::from_str(text),
            path: None,
            line_endings: LineEndingProfile::detect(text),
            encoding: Encoding::Utf8,
            bom,
            revision: Revision::INITIAL,
            content_fingerprint: fingerprint,
            saved_content_fingerprint: fingerprint,
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

    /// Returns the trusted destination expectation captured at load or save.
    ///
    /// External-change detection compares live observations with this baseline.
    /// Keep Editing and ordinary editing never mutate it; only a successful save
    /// or a new load may replace it.
    pub const fn saved_target(&self) -> Option<TargetExpectation> {
        self.saved_target
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

    /// Returns the maximum UTF-8 rope length after reserving serialized BOM bytes.
    pub const fn maximum_text_bytes(&self) -> usize {
        MAX_DOCUMENT_BYTES - self.bom.as_bytes().len()
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

    /// Returns whether serialized content differs from the last committed snapshot.
    ///
    /// Revisions remain monotonic through Undo and Redo, so dirty identity is
    /// tracked independently from revision identity.
    pub fn is_dirty(&self) -> bool {
        self.content_fingerprint != self.saved_content_fingerprint
    }

    /// Marks recovered crash-recovery content as dirty without changing bytes.
    ///
    /// Restored work must never appear clean until the user saves (FR-066). A
    /// load from recovery bytes would otherwise match the saved fingerprint and
    /// suppress the dirty prompt and recovery scheduling.
    pub fn mark_recovered_dirty(&mut self) {
        // Sentinel baseline cannot match a real document fingerprint in practice
        // and is not a committed path; Save establishes a true saved baseline.
        self.saved_content_fingerprint =
            ContentFingerprint::from_bytes(b"\0noter-recovery-unsaved-baseline");
        self.saved_target = None;
    }

    /// Replaces authoritative text and advances the revision exactly once when changed.
    ///
    /// # Errors
    ///
    /// Returns [`NoterError::RevisionExhausted`] if the monotonic counter cannot
    /// advance without wrapping, or [`NoterError::Edit`] when the replacement
    /// exceeds the BOM-aware document ceiling.
    pub fn replace_text(&mut self, text: &str) -> Result<bool, NoterError> {
        let maximum = self.maximum_text_bytes();
        self.replace_text_with_maximum(text, maximum)
    }

    fn replace_text_with_maximum(
        &mut self,
        text: &str,
        maximum: usize,
    ) -> Result<bool, NoterError> {
        if text.len() > maximum {
            return Err(NoterError::Edit(EditError::ResultTooLarge {
                projected: text.len(),
                maximum,
            }));
        }
        let before = self.rope.to_string();
        let Some(transaction) = EditTransaction::between(
            self.revision,
            &before,
            text,
            Selection::caret(before.len()),
            Selection::caret(text.len()),
            EditOrigin::Programmatic,
            EditTimestamp::default(),
        )?
        else {
            return Ok(false);
        };
        match self.apply_transaction(&transaction) {
            Ok(_) => Ok(true),
            Err(EditError::RevisionExhausted) => Err(NoterError::RevisionExhausted),
            Err(error) => Err(error.into()),
        }
    }

    /// Applies one validated source transaction atomically.
    ///
    /// The base revision, ordered UTF-8 byte ranges, expected removed text, and
    /// before and after selections are all validated against cloned content
    /// before the authoritative rope or revision changes. Success returns the
    /// exact inverse required for Undo.
    ///
    /// # Errors
    ///
    /// Returns [`EditError`] for a stale revision, invalid boundary, overlapping
    /// edit, removed-text mismatch, invalid selection, or exhausted revision.
    pub fn apply_transaction(
        &mut self,
        transaction: &EditTransaction,
    ) -> Result<AppliedTransaction, EditError> {
        let (replacement, applied) =
            transaction.apply_to(&self.rope, self.revision, self.maximum_text_bytes())?;
        let replacement_text = replacement.to_string();
        let replacement_line_endings = LineEndingProfile::detect(&replacement_text);
        self.rope = replacement;
        self.line_endings = replacement_line_endings;
        self.revision = applied.revision();
        self.content_fingerprint = serialized_fingerprint(self.bom, &self.rope);
        Ok(applied)
    }

    /// Saves the current revision to its trusted existing path.
    ///
    /// Dirty state updates only when the exact current revision commits. Undo or
    /// Redo can later return to those saved bytes without reusing a revision.
    /// Conflict, proven failure, and unknown commit state remain explicit.
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
        let prepared = self.prepare_save_as(path)?;
        self.save_prepared_as(prepared)
    }

    /// Inspects and versions a user-selected Save As destination.
    ///
    /// Keep this value unchanged while any hard-link confirmation is visible.
    /// The later save uses this exact expectation instead of trusting a second
    /// inspection after the user responds.
    ///
    /// # Errors
    ///
    /// Returns a storage error if the path cannot be inspected safely,
    /// [`NoterError::MissingFileBaseline`] when the selected path is the current
    /// path without a trusted baseline, or [`NoterError::UnsupportedTarget`]
    /// for a special final entry.
    pub fn prepare_save_as(&self, path: impl AsRef<Path>) -> Result<PreparedSaveAs, NoterError> {
        let path = path.as_ref();
        if self.path.as_deref() == Some(path) {
            let expected = self.saved_target.ok_or(NoterError::MissingFileBaseline)?;
            return Ok(PreparedSaveAs {
                path: path.to_path_buf(),
                expected,
                adopt_path: false,
            });
        }
        let expected = match inspect_target(path, SaveStage::InspectInitial)? {
            TargetState::Missing => TargetExpectation::Missing,
            TargetState::Regular(observation) => TargetExpectation::Existing(observation),
            TargetState::Special(kind) => {
                return Err(NoterError::UnsupportedTarget(format!("{kind:?}")));
            }
        };
        Ok(PreparedSaveAs {
            path: path.to_path_buf(),
            expected,
            adopt_path: true,
        })
    }

    /// Saves using the exact destination version captured by
    /// [`Self::prepare_save_as`].
    ///
    /// # Errors
    ///
    /// Returns [`NoterError::HardLinkedTarget`] when explicit confirmation is
    /// required.
    pub fn save_prepared_as(
        &mut self,
        prepared: PreparedSaveAs,
    ) -> Result<SaveOutcome, NoterError> {
        self.save_prepared_as_with_hard_link_policy(prepared, false)
    }

    /// Saves a prepared destination after explicit hard-link confirmation.
    ///
    /// The captured expectation is deliberately not refreshed. If the selected
    /// entry changed while the confirmation was visible, the protocol returns a
    /// conflict and preserves the newer external object.
    ///
    /// # Errors
    ///
    /// Returns the same protocol errors as [`Self::save_prepared_as`].
    pub fn save_prepared_as_confirming_hard_link_replacement(
        &mut self,
        prepared: PreparedSaveAs,
    ) -> Result<SaveOutcome, NoterError> {
        self.save_prepared_as_with_hard_link_policy(prepared, true)
    }

    fn save_prepared_as_with_hard_link_policy(
        &mut self,
        prepared: PreparedSaveAs,
        hard_link_confirmed: bool,
    ) -> Result<SaveOutcome, NoterError> {
        require_hard_link_confirmation(prepared.expected, hard_link_confirmed)?;
        Ok(self.save_to_expectation(prepared.path, prepared.expected, prepared.adopt_path))
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
            self.saved_content_fingerprint = self.content_fingerprint;
            self.saved_target = Some(TargetExpectation::Existing(*observation));
            if adopt_path {
                self.path = Some(path);
            }
        }
        outcome
    }
}

fn serialized_fingerprint(bom: Bom, rope: &Rope) -> ContentFingerprint {
    let mut hasher = blake3::Hasher::new();
    hasher.update(bom.as_bytes());
    for chunk in rope.chunks() {
        hasher.update(chunk.as_bytes());
    }
    ContentFingerprint::new(*hasher.finalize().as_bytes())
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
    use crate::core::edit::{TextEdit, TextRange};
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
    fn mark_recovered_dirty_keeps_bytes_and_sets_dirty() -> Result<(), NoterError> {
        let mut document = Document::from_bytes(b"recovered text")?;
        assert!(!document.is_dirty());
        let before = document.to_bytes();
        document.mark_recovered_dirty();
        assert!(document.is_dirty());
        assert_eq!(document.to_bytes(), before);
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
    fn serialized_document_ceiling_is_exact_and_includes_the_bom() {
        let exact = vec![b'x'; MAX_DOCUMENT_BYTES];
        let document = Document::from_bytes(&exact).expect("the exact ceiling should load");
        assert_eq!(document.rope().len_bytes(), MAX_DOCUMENT_BYTES);
        assert_eq!(document.to_bytes().len(), MAX_DOCUMENT_BYTES);
        drop(document);
        drop(exact);

        let oversized = vec![b'x'; MAX_DOCUMENT_BYTES + 1];
        assert!(matches!(
            Document::from_bytes(&oversized),
            Err(NoterError::DocumentTooLarge {
                actual,
                maximum: MAX_DOCUMENT_BYTES,
            }) if actual == MAX_DOCUMENT_BYTES + 1
        ));

        let bom_document = Document::from_bytes(b"\xEF\xBB\xBFx").expect("BOM fixture should load");
        assert_eq!(
            bom_document.maximum_text_bytes(),
            MAX_DOCUMENT_BYTES - Bom::Utf8.as_bytes().len()
        );
    }

    #[test]
    fn authoritative_document_edit_rejects_growth_beyond_the_ceiling() {
        let initial = vec![b'x'; MAX_DOCUMENT_BYTES - 1];
        let mut document = Document::from_bytes(&initial).expect("bounded fixture should load");
        let append = EditTransaction::new(
            document.revision(),
            vec![TextEdit::replace(
                TextRange::new(initial.len(), initial.len()),
                "y",
                "",
            )],
            Selection::caret(initial.len()),
            Selection::caret(initial.len() + 1),
            EditOrigin::TextInput,
            EditTimestamp::default(),
        );
        document
            .apply_transaction(&append)
            .expect("an edit reaching the exact ceiling should apply");
        assert_eq!(document.rope().len_bytes(), MAX_DOCUMENT_BYTES);

        let overflow = EditTransaction::new(
            document.revision(),
            vec![TextEdit::replace(
                TextRange::new(MAX_DOCUMENT_BYTES, MAX_DOCUMENT_BYTES),
                "z",
                "",
            )],
            Selection::caret(MAX_DOCUMENT_BYTES),
            Selection::caret(MAX_DOCUMENT_BYTES + 1),
            EditOrigin::Paste,
            EditTimestamp::default(),
        );
        assert!(matches!(
            document.apply_transaction(&overflow),
            Err(EditError::ResultTooLarge {
                projected,
                maximum: MAX_DOCUMENT_BYTES,
            }) if projected == MAX_DOCUMENT_BYTES + 1
        ));
        assert_eq!(document.rope().len_bytes(), MAX_DOCUMENT_BYTES);
    }

    #[test]
    fn whole_text_replacement_limit_is_exact_and_rejects_before_diffing() {
        let mut exact_document = Document::new();

        let changed = exact_document
            .replace_text_with_maximum("four", 4)
            .expect("a replacement at the exact ceiling should succeed");
        assert!(changed);
        assert_eq!(exact_document.rope().to_string(), "four");
        assert!(exact_document.is_dirty());

        let mut document = Document::from_bytes(b"unchanged").expect("fixture should load");
        let revision = document.revision();

        assert!(matches!(
            document.replace_text_with_maximum("12345", 4),
            Err(NoterError::Edit(EditError::ResultTooLarge {
                projected,
                maximum: 4,
            })) if projected == 5
        ));
        assert_eq!(document.rope().to_string(), "unchanged");
        assert_eq!(document.revision(), revision);
        assert!(!document.is_dirty());
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
