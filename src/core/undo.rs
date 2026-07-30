//! Bounded undo and redo history for edit transactions.

use std::collections::VecDeque;
use std::time::Duration;

use thiserror::Error;

use super::document::Document;
use super::edit::{
    AppliedTransaction, EditError, EditIntent, EditTransaction, Selection, TextEdit, TextRange,
};
use super::revision::Revision;

/// Default maximum number of retained transactions.
pub const DEFAULT_MAX_HISTORY_TRANSACTIONS: usize = 1_024;
/// Default maximum source bytes retained across undo and redo.
pub const DEFAULT_MAX_HISTORY_BYTES: usize = 32 * 1024 * 1024;
/// Maximum elapsed time between direct edits in one Undo transaction.
pub const DEFAULT_EDIT_COALESCING_WINDOW: Duration = Duration::from_millis(750);
/// Maximum source bytes retained by one coalesced direct-edit transaction.
pub const DEFAULT_MAX_COALESCED_EDIT_BYTES: usize = 16_384;

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
    /// The inverse joined the immediately preceding compatible direct edit.
    Coalesced,
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

        let max_coalesced_bytes = self.limits.max_bytes.min(DEFAULT_MAX_COALESCED_EDIT_BYTES);
        let coalesced = self
            .undo
            .back()
            .and_then(|previous| coalesce_inverse(previous, &inverse, max_coalesced_bytes));
        let outcome = if let (Some(previous), Some(coalesced)) = (self.undo.back_mut(), coalesced) {
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(previous.retained_bytes())
                .saturating_add(coalesced.retained_bytes());
            *previous = coalesced;
            HistoryRecordOutcome::Coalesced
        } else {
            self.retained_bytes = self.retained_bytes.saturating_add(cost);
            self.undo.push_back(inverse);
            HistoryRecordOutcome::Stored
        };
        while self.undo.len() > self.limits.max_transactions
            || self.retained_bytes > self.limits.max_bytes
        {
            let Some(evicted) = self.undo.pop_front() else {
                self.retained_bytes = 0;
                break;
            };
            self.retained_bytes = self.retained_bytes.saturating_sub(evicted.retained_bytes());
        }
        outcome
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

fn coalesce_inverse(
    previous: &EditTransaction,
    current: &EditTransaction,
    max_retained_bytes: usize,
) -> Option<EditTransaction> {
    if previous.origin() != current.origin() || previous.intent() != current.intent() {
        return None;
    }
    if current.selection_after() != previous.selection_before() {
        return None;
    }
    let elapsed = current
        .observed_at()
        .elapsed()
        .checked_sub(previous.observed_at().elapsed())?;
    if elapsed > DEFAULT_EDIT_COALESCING_WINDOW {
        return None;
    }

    let [previous_edit] = previous.edits() else {
        return None;
    };
    let [current_edit] = current.edits() else {
        return None;
    };
    let combined_retained_bytes = previous
        .retained_bytes()
        .checked_add(current.retained_bytes())?;
    if combined_retained_bytes > max_retained_bytes {
        return None;
    }
    let edit = match current.intent() {
        EditIntent::Insert => coalesce_insert_inverse(previous_edit, current_edit)?,
        EditIntent::Backspace => coalesce_backspace_inverse(previous_edit, current_edit)?,
        EditIntent::Delete => coalesce_delete_inverse(previous_edit, current_edit)?,
        EditIntent::ReplaceSelection
        | EditIntent::Paste
        | EditIntent::Formatting
        | EditIntent::Replace
        | EditIntent::LineEndingConversion
        | EditIntent::Programmatic
        | EditIntent::Unclassified => return None,
    };
    let coalesced = EditTransaction::new_with_intent(
        current.base_revision(),
        vec![edit],
        current.selection_before(),
        previous.selection_after(),
        current.origin(),
        current.intent(),
        current.observed_at(),
    );
    debug_assert_eq!(coalesced.retained_bytes(), combined_retained_bytes);
    Some(coalesced)
}

fn coalesce_insert_inverse(previous: &TextEdit, current: &TextEdit) -> Option<TextEdit> {
    if !previous.inserted().is_empty()
        || previous.removed().is_empty()
        || !current.inserted().is_empty()
        || current.removed().is_empty()
        || previous.range().end() != current.range().start()
    {
        return None;
    }
    Some(TextEdit::replace(
        TextRange::new(previous.range().start(), current.range().end()),
        "",
        concatenate(previous.removed(), current.removed())?,
    ))
}

fn coalesce_backspace_inverse(previous: &TextEdit, current: &TextEdit) -> Option<TextEdit> {
    if previous.inserted().is_empty()
        || !previous.removed().is_empty()
        || previous.range().start() != previous.range().end()
        || current.inserted().is_empty()
        || !current.removed().is_empty()
        || current.range().start() != current.range().end()
        || current
            .range()
            .start()
            .checked_add(current.inserted().len())?
            != previous.range().start()
    {
        return None;
    }
    Some(TextEdit::replace(
        current.range(),
        concatenate(current.inserted(), previous.inserted())?,
        "",
    ))
}

fn coalesce_delete_inverse(previous: &TextEdit, current: &TextEdit) -> Option<TextEdit> {
    if previous.inserted().is_empty()
        || !previous.removed().is_empty()
        || previous.range().start() != previous.range().end()
        || current.inserted().is_empty()
        || !current.removed().is_empty()
        || current.range().start() != current.range().end()
        || previous.range().start() != current.range().start()
    {
        return None;
    }
    Some(TextEdit::replace(
        current.range(),
        concatenate(previous.inserted(), current.inserted())?,
        "",
    ))
}

fn concatenate(first: &str, second: &str) -> Option<String> {
    let capacity = first.len().checked_add(second.len())?;
    let mut combined = String::new();
    combined.try_reserve_exact(capacity).ok()?;
    combined.push_str(first);
    combined.push_str(second);
    Some(combined)
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
            EditOrigin::Programmatic,
            EditTimestamp::new(Duration::from_millis(1)),
        );
        let applied = document
            .apply_transaction(&transaction)
            .expect("test transaction should apply");
        history.record(applied)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_direct_change(
        document: &mut Document,
        history: &mut UndoHistory,
        range: TextRange,
        inserted: &str,
        removed: &str,
        before: Selection,
        after: Selection,
        origin: EditOrigin,
        observed_ms: u64,
    ) -> HistoryRecordOutcome {
        let transaction = EditTransaction::new(
            document.revision(),
            vec![TextEdit::replace(range, inserted, removed)],
            before,
            after,
            origin,
            EditTimestamp::new(Duration::from_millis(observed_ms)),
        );
        let applied = document
            .apply_transaction(&transaction)
            .expect("direct test transaction should apply");
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
    fn adjacent_typing_coalesces_with_utf8_and_round_trips_as_one_step() {
        let mut document = Document::new();
        let mut history = UndoHistory::default();

        for (range, inserted, before, after, time, expected) in [
            (
                TextRange::new(0, 0),
                "a",
                Selection::caret(0),
                Selection::caret(1),
                0,
                HistoryRecordOutcome::Stored,
            ),
            (
                TextRange::new(1, 1),
                "b",
                Selection::caret(1),
                Selection::caret(2),
                100,
                HistoryRecordOutcome::Coalesced,
            ),
            (
                TextRange::new(2, 2),
                "é",
                Selection::caret(2),
                Selection::caret(4),
                200,
                HistoryRecordOutcome::Coalesced,
            ),
        ] {
            assert_eq!(
                apply_direct_change(
                    &mut document,
                    &mut history,
                    range,
                    inserted,
                    "",
                    before,
                    after,
                    EditOrigin::TextInput,
                    time,
                ),
                expected
            );
        }

        assert_eq!(document.rope().to_string(), "abé");
        assert_eq!(history.len(), 1);
        assert_eq!(history.retained_bytes(), 4);
        let undone = history
            .undo(&mut document)
            .expect("coalesced typing should undo")
            .expect("one coalesced entry should exist");
        assert_eq!(document.rope().to_string(), "");
        assert_eq!(undone.selection(), Selection::caret(0));
        let redone = history
            .redo(&mut document)
            .expect("coalesced typing should redo")
            .expect("one coalesced redo entry should exist");
        assert_eq!(document.rope().to_string(), "abé");
        assert_eq!(redone.selection(), Selection::caret(4));
    }

    #[test]
    fn backspace_and_forward_delete_coalesce_independently() {
        let mut backspace_document =
            Document::from_bytes("abé".as_bytes()).expect("fixture should load");
        let mut backspace_history =
            UndoHistory::new(HistoryLimits::default(), backspace_document.revision());
        assert_eq!(
            apply_direct_change(
                &mut backspace_document,
                &mut backspace_history,
                TextRange::new(2, 4),
                "",
                "é",
                Selection::caret(4),
                Selection::caret(2),
                EditOrigin::TextInput,
                0,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(
            apply_direct_change(
                &mut backspace_document,
                &mut backspace_history,
                TextRange::new(1, 2),
                "",
                "b",
                Selection::caret(2),
                Selection::caret(1),
                EditOrigin::TextInput,
                100,
            ),
            HistoryRecordOutcome::Coalesced
        );
        backspace_history
            .undo(&mut backspace_document)
            .expect("backspaces should undo together");
        assert_eq!(backspace_document.rope().to_string(), "abé");

        let mut delete_document = Document::from_bytes(b"abc").expect("fixture should load");
        let mut delete_history =
            UndoHistory::new(HistoryLimits::default(), delete_document.revision());
        assert_eq!(
            apply_direct_change(
                &mut delete_document,
                &mut delete_history,
                TextRange::new(0, 1),
                "",
                "a",
                Selection::caret(0),
                Selection::caret(0),
                EditOrigin::TextInput,
                0,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(
            apply_direct_change(
                &mut delete_document,
                &mut delete_history,
                TextRange::new(0, 1),
                "",
                "b",
                Selection::caret(0),
                Selection::caret(0),
                EditOrigin::TextInput,
                100,
            ),
            HistoryRecordOutcome::Coalesced
        );
        delete_history
            .undo(&mut delete_document)
            .expect("forward deletes should undo together");
        assert_eq!(delete_document.rope().to_string(), "abc");
    }

    #[test]
    fn coalescing_shape_predicates_have_independent_boundaries() {
        let valid_insert_previous = TextEdit::replace(TextRange::new(0, 1), "", "a");
        let valid_insert_current = TextEdit::replace(TextRange::new(1, 2), "", "b");
        assert!(coalesce_insert_inverse(&valid_insert_previous, &valid_insert_current).is_some());
        for (previous, current) in [
            (
                TextEdit::replace(TextRange::new(0, 1), "x", "a"),
                valid_insert_current.clone(),
            ),
            (
                TextEdit::replace(TextRange::new(0, 1), "", ""),
                valid_insert_current,
            ),
            (
                valid_insert_previous.clone(),
                TextEdit::replace(TextRange::new(1, 2), "x", "b"),
            ),
            (
                valid_insert_previous,
                TextEdit::replace(TextRange::new(1, 2), "", ""),
            ),
        ] {
            assert!(coalesce_insert_inverse(&previous, &current).is_none());
        }

        let valid_backspace_previous = TextEdit::replace(TextRange::new(1, 1), "b", "");
        let valid_backspace_current = TextEdit::replace(TextRange::new(0, 0), "a", "");
        assert!(
            coalesce_backspace_inverse(&valid_backspace_previous, &valid_backspace_current)
                .is_some()
        );
        for (previous, current) in [
            (
                TextEdit::replace(TextRange::new(1, 1), "", ""),
                valid_backspace_current.clone(),
            ),
            (
                TextEdit::replace(TextRange::new(1, 1), "b", "x"),
                valid_backspace_current.clone(),
            ),
            (
                TextEdit::replace(TextRange::new(1, 2), "b", ""),
                valid_backspace_current,
            ),
            (
                valid_backspace_previous.clone(),
                TextEdit::replace(TextRange::new(1, 1), "", ""),
            ),
            (
                valid_backspace_previous.clone(),
                TextEdit::replace(TextRange::new(0, 0), "a", "x"),
            ),
            (
                valid_backspace_previous,
                TextEdit::replace(TextRange::new(0, 1), "a", ""),
            ),
        ] {
            assert!(coalesce_backspace_inverse(&previous, &current).is_none());
        }

        let valid_delete_previous = TextEdit::replace(TextRange::new(0, 0), "a", "");
        let valid_delete_current = TextEdit::replace(TextRange::new(0, 0), "b", "");
        assert!(coalesce_delete_inverse(&valid_delete_previous, &valid_delete_current).is_some());
        for (previous, current) in [
            (
                TextEdit::replace(TextRange::new(0, 0), "", ""),
                valid_delete_current.clone(),
            ),
            (
                TextEdit::replace(TextRange::new(0, 0), "a", "x"),
                valid_delete_current.clone(),
            ),
            (
                TextEdit::replace(TextRange::new(0, 1), "a", ""),
                valid_delete_current,
            ),
            (
                valid_delete_previous.clone(),
                TextEdit::replace(TextRange::new(0, 0), "", ""),
            ),
            (
                valid_delete_previous.clone(),
                TextEdit::replace(TextRange::new(0, 0), "b", "x"),
            ),
            (
                valid_delete_previous,
                TextEdit::replace(TextRange::new(0, 1), "b", ""),
            ),
        ] {
            assert!(coalesce_delete_inverse(&previous, &current).is_none());
        }
    }

    #[test]
    fn coalescing_breaks_at_time_and_origin_boundaries() {
        let limits = HistoryLimits {
            max_transactions: 16,
            max_bytes: 2,
        };
        let mut document = Document::new();
        let mut history = UndoHistory::new(limits, document.revision());
        for (range, inserted, before, after, origin, time) in [
            (
                TextRange::new(0, 0),
                "a",
                Selection::caret(0),
                Selection::caret(1),
                EditOrigin::TextInput,
                0,
            ),
            (
                TextRange::new(1, 1),
                "b",
                Selection::caret(1),
                Selection::caret(2),
                EditOrigin::TextInput,
                751,
            ),
            (
                TextRange::new(2, 2),
                "c",
                Selection::caret(2),
                Selection::caret(3),
                EditOrigin::MarkdownInput,
                800,
            ),
            (
                TextRange::new(3, 3),
                "d",
                Selection::caret(3),
                Selection::caret(4),
                EditOrigin::Paste,
                850,
            ),
        ] {
            assert_eq!(
                apply_direct_change(
                    &mut document,
                    &mut history,
                    range,
                    inserted,
                    "",
                    before,
                    after,
                    origin,
                    time,
                ),
                HistoryRecordOutcome::Stored
            );
        }
        assert_eq!(document.rope().to_string(), "abcd");
        assert_eq!(history.len(), 2);
        history
            .undo(&mut document)
            .expect("paste should remain an isolated newest step");
        assert_eq!(document.rope().to_string(), "abc");
    }

    #[test]
    fn coalescing_breaks_at_clock_selection_and_intent_boundaries() {
        let mut moved_document = Document::new();
        let mut moved_history = UndoHistory::default();
        apply_direct_change(
            &mut moved_document,
            &mut moved_history,
            TextRange::new(0, 0),
            "a",
            "",
            Selection::caret(0),
            Selection::caret(1),
            EditOrigin::TextInput,
            100,
        );
        assert_eq!(
            apply_direct_change(
                &mut moved_document,
                &mut moved_history,
                TextRange::new(1, 1),
                "x",
                "",
                Selection::caret(1),
                Selection::caret(2),
                EditOrigin::TextInput,
                50,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(moved_history.len(), 2);

        let mut selection_document = Document::new();
        let mut selection_history = UndoHistory::default();
        apply_direct_change(
            &mut selection_document,
            &mut selection_history,
            TextRange::new(0, 0),
            "a",
            "",
            Selection::caret(0),
            Selection::caret(1),
            EditOrigin::TextInput,
            0,
        );
        assert_eq!(
            apply_direct_change(
                &mut selection_document,
                &mut selection_history,
                TextRange::new(0, 0),
                "x",
                "",
                Selection::caret(0),
                Selection::caret(1),
                EditOrigin::TextInput,
                100,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(selection_document.rope().to_string(), "xa");
        assert_eq!(selection_history.len(), 2);

        let mut intent_document = Document::new();
        let mut intent_history = UndoHistory::default();
        apply_direct_change(
            &mut intent_document,
            &mut intent_history,
            TextRange::new(0, 0),
            "a",
            "",
            Selection::caret(0),
            Selection::caret(1),
            EditOrigin::TextInput,
            0,
        );
        assert_eq!(
            apply_direct_change(
                &mut intent_document,
                &mut intent_history,
                TextRange::new(0, 1),
                "",
                "a",
                Selection::caret(1),
                Selection::caret(0),
                EditOrigin::TextInput,
                100,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(intent_document.rope().to_string(), "");
        assert_eq!(intent_history.len(), 2);
    }

    #[test]
    fn coalescing_window_is_inclusive_and_never_evades_history_byte_limits() {
        assert_eq!(DEFAULT_EDIT_COALESCING_WINDOW, Duration::from_millis(750));
        let mut timed_document = Document::new();
        let mut timed_history = UndoHistory::default();
        assert_eq!(
            apply_direct_change(
                &mut timed_document,
                &mut timed_history,
                TextRange::new(0, 0),
                "a",
                "",
                Selection::caret(0),
                Selection::caret(1),
                EditOrigin::TextInput,
                0,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(
            apply_direct_change(
                &mut timed_document,
                &mut timed_history,
                TextRange::new(1, 1),
                "b",
                "",
                Selection::caret(1),
                Selection::caret(2),
                EditOrigin::TextInput,
                750,
            ),
            HistoryRecordOutcome::Coalesced
        );
        assert_eq!(timed_history.len(), 1);

        let limits = HistoryLimits {
            max_transactions: 8,
            max_bytes: 1,
        };
        let mut bounded_document = Document::new();
        let mut bounded_history = UndoHistory::new(limits, bounded_document.revision());
        apply_direct_change(
            &mut bounded_document,
            &mut bounded_history,
            TextRange::new(0, 0),
            "a",
            "",
            Selection::caret(0),
            Selection::caret(1),
            EditOrigin::TextInput,
            0,
        );
        assert_eq!(
            apply_direct_change(
                &mut bounded_document,
                &mut bounded_history,
                TextRange::new(1, 1),
                "b",
                "",
                Selection::caret(1),
                Selection::caret(2),
                EditOrigin::TextInput,
                100,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(bounded_history.len(), 1);
        assert_eq!(bounded_history.retained_bytes(), 1);
        bounded_history
            .undo(&mut bounded_document)
            .expect("the newest bounded edit should remain undoable");
        assert_eq!(bounded_document.rope().to_string(), "a");
    }

    #[test]
    fn coalescing_byte_ceiling_is_exact_and_checked_before_combining_text() {
        let first = "a".repeat(DEFAULT_MAX_COALESCED_EDIT_BYTES / 2);
        let second = "b".repeat(DEFAULT_MAX_COALESCED_EDIT_BYTES / 2);
        let mut document = Document::new();
        let mut history = UndoHistory::default();

        assert_eq!(
            apply_direct_change(
                &mut document,
                &mut history,
                TextRange::new(0, 0),
                &first,
                "",
                Selection::caret(0),
                Selection::caret(first.len()),
                EditOrigin::TextInput,
                0,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(
            apply_direct_change(
                &mut document,
                &mut history,
                TextRange::new(first.len(), first.len()),
                &second,
                "",
                Selection::caret(first.len()),
                Selection::caret(first.len() + second.len()),
                EditOrigin::TextInput,
                1,
            ),
            HistoryRecordOutcome::Coalesced
        );
        assert_eq!(history.len(), 1);
        assert_eq!(history.retained_bytes(), DEFAULT_MAX_COALESCED_EDIT_BYTES);

        let end = first.len() + second.len();
        assert_eq!(
            apply_direct_change(
                &mut document,
                &mut history,
                TextRange::new(end, end),
                "c",
                "",
                Selection::caret(end),
                Selection::caret(end + 1),
                EditOrigin::TextInput,
                2,
            ),
            HistoryRecordOutcome::Stored
        );
        assert_eq!(history.len(), 2);
        assert_eq!(
            history.retained_bytes(),
            DEFAULT_MAX_COALESCED_EDIT_BYTES + 1
        );
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
