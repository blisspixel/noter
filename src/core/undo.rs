//! Bounded undo and redo history for edit transactions.

use std::collections::VecDeque;

use thiserror::Error;

use super::document::Document;
use super::edit::{AppliedTransaction, EditError, EditTransaction, Selection};
use super::revision::Revision;

/// Default maximum number of retained transactions.
pub const DEFAULT_MAX_HISTORY_TRANSACTIONS: usize = 1_024;
/// Default maximum source bytes retained across undo and redo.
pub const DEFAULT_MAX_HISTORY_BYTES: usize = 32 * 1024 * 1024;

/// Independent count and memory ceilings for edit history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HistoryLimits {
    /// Maximum number of transactions across undo and redo.
    pub max_transactions: usize,
    /// Maximum inserted plus removed source bytes retained in memory.
    pub max_bytes: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            max_transactions: DEFAULT_MAX_HISTORY_TRANSACTIONS,
            max_bytes: DEFAULT_MAX_HISTORY_BYTES,
        }
    }
}

/// Result of recording a newly applied user transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HistoryRecordOutcome {
    /// The inverse was retained and can be undone.
    Stored,
    /// The transaction exceeded a configured ceiling, so all now-stale history
    /// was cleared instead of retaining an unbounded inverse.
    ClearedForOversizedTransaction,
}

/// Result of an Undo or Redo command.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HistoryApplyOutcome {
    revision: Revision,
    selection: Selection,
}

impl HistoryApplyOutcome {
    /// Returns the revision produced by Undo or Redo.
    pub const fn revision(self) -> Revision {
        self.revision
    }

    /// Returns the exact directional selection restored by the command.
    pub const fn selection(self) -> Selection {
        self.selection
    }
}

/// Undo history failures that leave document content unchanged.
#[derive(Clone, PartialEq, Eq, Error, Debug)]
pub enum HistoryError {
    /// Content changed without recording the corresponding transaction.
    #[error("edit history expected {expected:?}, but the document is {actual:?}")]
    OutOfSync {
        /// Revision last observed by history.
        expected: Revision,
        /// Current authoritative revision.
        actual: Revision,
    },
    /// A retained inverse failed transaction validation.
    #[error("failed to apply retained edit history: {0}")]
    Edit(#[from] EditError),
}

/// Revision-aware undo and redo stacks bounded by count and retained bytes.
#[derive(Debug)]
pub struct UndoHistory {
    undo: VecDeque<EditTransaction>,
    redo: Vec<EditTransaction>,
    // An exact inverse swaps inserted and removed source, so moving a
    // transaction between stacks does not change this total.
    retained_bytes: usize,
    expected_revision: Revision,
    limits: HistoryLimits,
}

impl UndoHistory {
    /// Creates empty history synchronized to `revision`.
    pub const fn new(limits: HistoryLimits, revision: Revision) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            retained_bytes: 0,
            expected_revision: revision,
            limits,
        }
    }

    /// Clears all entries and synchronizes history to a replacement document.
    pub fn reset(&mut self, revision: Revision) {
        self.undo.clear();
        self.redo.clear();
        self.retained_bytes = 0;
        self.expected_revision = revision;
    }

    /// Returns whether an Undo command has a retained transaction.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Returns whether a Redo command has a retained transaction.
    pub const fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Returns the number of retained transactions across both stacks.
    pub fn len(&self) -> usize {
        self.undo.len() + self.redo.len()
    }

    /// Returns whether both history stacks are empty.
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }

    /// Returns the exact inserted plus removed source bytes retained in memory.
    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    /// Records the exact inverse of a newly applied user edit.
    ///
    /// A new branch clears Redo. If another mutation bypassed this history, all
    /// prior entries are discarded before the new inverse is accepted.
    pub fn record(&mut self, applied: AppliedTransaction) -> HistoryRecordOutcome {
        if self.expected_revision != applied.base_revision() {
            self.reset(applied.base_revision());
        }
        self.clear_redo();
        self.expected_revision = applied.revision();
        let inverse = applied.into_inverse();
        let cost = inverse.retained_bytes();
        if self.limits.max_transactions == 0 || cost > self.limits.max_bytes {
            self.reset(self.expected_revision);
            return HistoryRecordOutcome::ClearedForOversizedTransaction;
        }

        self.retained_bytes = self.retained_bytes.saturating_add(cost);
        self.undo.push_back(inverse);
        while self.undo.len() > self.limits.max_transactions
            || self.retained_bytes > self.limits.max_bytes
        {
            let Some(evicted) = self.undo.pop_front() else {
                self.retained_bytes = 0;
                break;
            };
            self.retained_bytes = self.retained_bytes.saturating_sub(evicted.retained_bytes());
        }
        HistoryRecordOutcome::Stored
    }

    /// Applies the newest inverse and moves its exact inverse to Redo.
    ///
    /// # Errors
    ///
    /// Returns [`HistoryError::OutOfSync`] when the document changed outside
    /// this history, or [`HistoryError::Edit`] if retained data fails validation.
    pub fn undo(
        &mut self,
        document: &mut Document,
    ) -> Result<Option<HistoryApplyOutcome>, HistoryError> {
        self.require_revision(document.revision())?;
        let Some(transaction) = self.undo.pop_back() else {
            return Ok(None);
        };
        let rebased = transaction.rebased(document.revision());
        match document.apply_transaction(&rebased) {
            Ok(applied) => {
                let outcome = HistoryApplyOutcome {
                    revision: applied.revision(),
                    selection: applied.selection(),
                };
                let inverse = applied.into_inverse();
                self.redo.push(inverse);
                self.expected_revision = outcome.revision;
                Ok(Some(outcome))
            }
            Err(error) => {
                self.undo.push_back(transaction);
                Err(error.into())
            }
        }
    }

    /// Reapplies the newest Redo entry and moves its exact inverse to Undo.
    ///
    /// # Errors
    ///
    /// Returns the same defensive errors as [`Self::undo`].
    pub fn redo(
        &mut self,
        document: &mut Document,
    ) -> Result<Option<HistoryApplyOutcome>, HistoryError> {
        self.require_revision(document.revision())?;
        let Some(transaction) = self.redo.pop() else {
            return Ok(None);
        };
        let rebased = transaction.rebased(document.revision());
        match document.apply_transaction(&rebased) {
            Ok(applied) => {
                let outcome = HistoryApplyOutcome {
                    revision: applied.revision(),
                    selection: applied.selection(),
                };
                let inverse = applied.into_inverse();
                self.undo.push_back(inverse);
                self.expected_revision = outcome.revision;
                Ok(Some(outcome))
            }
            Err(error) => {
                self.redo.push(transaction);
                Err(error.into())
            }
        }
    }

    fn require_revision(&mut self, actual: Revision) -> Result<(), HistoryError> {
        if self.expected_revision == actual {
            return Ok(());
        }
        let expected = self.expected_revision;
        self.reset(actual);
        Err(HistoryError::OutOfSync { expected, actual })
    }

    fn clear_redo(&mut self) {
        for transaction in self.redo.drain(..) {
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(transaction.retained_bytes());
        }
    }
}

impl Default for UndoHistory {
    fn default() -> Self {
        Self::new(HistoryLimits::default(), Revision::INITIAL)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use super::*;
    use crate::core::edit::{
        EditOrigin, EditTimestamp, EditTransaction, Selection, TextEdit, TextRange,
    };
    use crate::core::save::SaveOutcome;
    use tempfile::tempdir;

    fn apply_change(
        document: &mut Document,
        history: &mut UndoHistory,
        range: TextRange,
        inserted: &str,
        removed: &str,
        before: Selection,
        after: Selection,
    ) -> HistoryRecordOutcome {
        let transaction = EditTransaction::new(
            document.revision(),
            vec![TextEdit::replace(range, inserted, removed)],
            before,
            after,
            EditOrigin::TextInput,
            EditTimestamp::new(Duration::from_millis(1)),
        );
        let applied = document
            .apply_transaction(&transaction)
            .expect("test transaction should apply");
        history.record(applied)
    }

    #[test]
    fn default_limits_and_empty_queries_are_exact() {
        assert_eq!(DEFAULT_MAX_HISTORY_TRANSACTIONS, 1_024);
        assert_eq!(DEFAULT_MAX_HISTORY_BYTES, 33_554_432);
        assert_eq!(
            HistoryLimits::default(),
            HistoryLimits {
                max_transactions: 1_024,
                max_bytes: 33_554_432,
            }
        );

        let history = UndoHistory::default();
        assert!(!history.can_undo());
        assert!(!history.can_redo());
        assert!(history.is_empty());
        assert_eq!(history.len(), 0);
        assert_eq!(history.retained_bytes(), 0);
    }

    #[test]
    fn history_queries_include_both_undo_and_redo_stacks() {
        let mut document = Document::new();
        let mut history = UndoHistory::default();
        for (offset, character) in [(0, "a"), (1, "b")] {
            apply_change(
                &mut document,
                &mut history,
                TextRange::new(offset, offset),
                character,
                "",
                Selection::caret(offset),
                Selection::caret(offset + 1),
            );
        }
        assert!(history.can_undo());
        assert!(!history.can_redo());
        assert!(!history.is_empty());
        assert_eq!(history.len(), 2);
        assert_eq!(history.retained_bytes(), 2);

        history.undo(&mut document).expect("undo should work");
        assert!(history.can_undo());
        assert!(history.can_redo());
        assert!(!history.is_empty());
        assert_eq!(history.len(), 2);
        assert_eq!(history.retained_bytes(), 2);
    }

    #[test]
    fn undo_and_redo_restore_content_selection_and_monotonic_revisions() {
        let mut document = Document::from_bytes(b"abc").expect("fixture should load");
        let mut history = UndoHistory::default();
        apply_change(
            &mut document,
            &mut history,
            TextRange::new(1, 2),
            "B",
            "b",
            Selection::new(2, 1),
            Selection::new(1, 2),
        );
        assert_eq!(document.rope().to_string(), "aBc");
        assert_eq!(document.revision(), Revision::new(1));
        assert!(document.is_dirty());

        let undone = history
            .undo(&mut document)
            .expect("undo should validate")
            .expect("an undo entry should exist");
        assert_eq!(document.rope().to_string(), "abc");
        assert_eq!(document.revision(), Revision::new(2));
        assert_eq!(undone.selection(), Selection::new(2, 1));
        assert!(!document.is_dirty());

        let redone = history
            .redo(&mut document)
            .expect("redo should validate")
            .expect("a redo entry should exist");
        assert_eq!(document.rope().to_string(), "aBc");
        assert_eq!(document.revision(), Revision::new(3));
        assert_eq!(redone.selection(), Selection::new(1, 2));
        assert!(document.is_dirty());
    }

    #[test]
    fn count_limit_evicts_oldest_entries_only() {
        let limits = HistoryLimits {
            max_transactions: 2,
            max_bytes: usize::MAX,
        };
        let mut document = Document::new();
        let mut history = UndoHistory::new(limits, document.revision());

        for (offset, character) in [(0, "a"), (1, "b"), (2, "c")] {
            assert_eq!(
                apply_change(
                    &mut document,
                    &mut history,
                    TextRange::new(offset, offset),
                    character,
                    "",
                    Selection::caret(offset),
                    Selection::caret(offset + 1),
                ),
                HistoryRecordOutcome::Stored
            );
        }
        assert_eq!(history.len(), 2);

        history
            .undo(&mut document)
            .expect("latest undo should work");
        history
            .undo(&mut document)
            .expect("second undo should work");
        assert_eq!(history.undo(&mut document), Ok(None));
        assert_eq!(document.rope().to_string(), "a");
    }

    #[test]
    fn oversized_edit_clears_history_that_could_no_longer_apply() {
        let limits = HistoryLimits {
            max_transactions: 8,
            max_bytes: 3,
        };
        let mut document = Document::new();
        let mut history = UndoHistory::new(limits, document.revision());
        assert_eq!(
            apply_change(
                &mut document,
                &mut history,
                TextRange::new(0, 0),
                "a",
                "",
                Selection::caret(0),
                Selection::caret(1),
            ),
            HistoryRecordOutcome::Stored
        );

        assert_eq!(
            apply_change(
                &mut document,
                &mut history,
                TextRange::new(1, 1),
                "long",
                "",
                Selection::caret(1),
                Selection::caret(5),
            ),
            HistoryRecordOutcome::ClearedForOversizedTransaction
        );
        assert!(history.is_empty());
        assert_eq!(history.retained_bytes(), 0);
        assert_eq!(history.undo(&mut document), Ok(None));
        assert_eq!(document.rope().to_string(), "along");
    }

    #[test]
    fn exact_byte_limit_is_retained_and_oldest_entry_is_evicted_only_afterward() {
        let limits = HistoryLimits {
            max_transactions: 8,
            max_bytes: 2,
        };
        let mut document = Document::new();
        let mut history = UndoHistory::new(limits, document.revision());

        assert_eq!(
            apply_change(
                &mut document,
                &mut history,
                TextRange::new(0, 0),
                "ab",
                "",
                Selection::caret(0),
                Selection::caret(2),
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(history.len(), 1);
        assert_eq!(history.retained_bytes(), 2);

        assert_eq!(
            apply_change(
                &mut document,
                &mut history,
                TextRange::new(2, 2),
                "c",
                "",
                Selection::caret(2),
                Selection::caret(3),
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(history.len(), 1);
        assert_eq!(history.retained_bytes(), 1);
        history
            .undo(&mut document)
            .expect("newest edit should remain");
        assert_eq!(document.rope().to_string(), "ab");
        assert_eq!(history.retained_bytes(), 1);
    }

    #[test]
    fn new_edit_after_undo_discards_redo_branch() {
        let mut document = Document::new();
        let mut history = UndoHistory::default();
        apply_change(
            &mut document,
            &mut history,
            TextRange::new(0, 0),
            "a",
            "",
            Selection::caret(0),
            Selection::caret(1),
        );
        history.undo(&mut document).expect("undo should work");
        assert!(history.can_redo());

        apply_change(
            &mut document,
            &mut history,
            TextRange::new(0, 0),
            "b",
            "",
            Selection::caret(0),
            Selection::caret(1),
        );
        assert!(!history.can_redo());
        assert_eq!(history.redo(&mut document), Ok(None));
        assert_eq!(document.rope().to_string(), "b");
    }

    #[test]
    fn unrecorded_document_change_rejects_and_clears_history() {
        let mut document = Document::new();
        let mut history = UndoHistory::default();
        apply_change(
            &mut document,
            &mut history,
            TextRange::new(0, 0),
            "a",
            "",
            Selection::caret(0),
            Selection::caret(1),
        );
        document
            .replace_text("external")
            .expect("unrecorded fixture edit should apply");
        let before = document.rope().to_string();

        assert!(matches!(
            history.undo(&mut document),
            Err(HistoryError::OutOfSync { .. })
        ));
        assert_eq!(document.rope().to_string(), before);
        assert!(history.is_empty());
    }

    #[test]
    fn undo_and_redo_compare_content_identity_to_the_latest_save() {
        let directory = tempdir().expect("temporary directory should be available");
        let path = directory.path().join("note.txt");
        fs::write(&path, b"original").expect("fixture should be writable");
        let mut document = Document::from_path(&path).expect("fixture should load");
        let mut history = UndoHistory::default();
        apply_change(
            &mut document,
            &mut history,
            TextRange::new(0, 8),
            "saved",
            "original",
            Selection::caret(8),
            Selection::caret(5),
        );
        assert!(matches!(
            document.save().expect("save should execute"),
            SaveOutcome::Committed { .. }
        ));
        assert!(!document.is_dirty());

        history.undo(&mut document).expect("undo should work");
        assert_eq!(document.rope().to_string(), "original");
        assert!(document.is_dirty());

        history.redo(&mut document).expect("redo should work");
        assert_eq!(document.rope().to_string(), "saved");
        assert!(!document.is_dirty());
    }
}
