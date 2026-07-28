//! Revision-checked, reversible text transactions.

use std::time::Duration;

use ropey::Rope;
use thiserror::Error;

use super::revision::Revision;

/// A half-open UTF-8 byte range in document source.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TextRange {
    start: usize,
    end: usize,
}

impl TextRange {
    /// Creates a half-open byte range without consulting document content.
    ///
    /// Applying a transaction validates order, bounds, and UTF-8 boundaries
    /// before any content changes.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Returns the inclusive start byte offset.
    pub const fn start(self) -> usize {
        self.start
    }

    /// Returns the exclusive end byte offset.
    pub const fn end(self) -> usize {
        self.end
    }

    /// Returns the number of source bytes covered by this range.
    const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// A directional source selection measured in UTF-8 byte offsets.
///
/// `anchor` remains fixed while `active` is the moving caret. Keeping both
/// positions preserves selection direction across undo and redo.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Selection {
    anchor: usize,
    active: usize,
}

impl Selection {
    /// Creates a directional selection.
    pub const fn new(anchor: usize, active: usize) -> Self {
        Self { anchor, active }
    }

    /// Creates an empty selection at one caret position.
    pub const fn caret(position: usize) -> Self {
        Self::new(position, position)
    }

    /// Returns the fixed selection endpoint.
    pub const fn anchor(self) -> usize {
        self.anchor
    }

    /// Returns the active caret endpoint.
    pub const fn active(self) -> usize {
        self.active
    }

    /// Returns the ordered source range covered by the selection.
    pub fn ordered_range(self) -> TextRange {
        TextRange::new(self.anchor.min(self.active), self.anchor.max(self.active))
    }
}

/// Monotonic observation time supplied by the editor adapter.
///
/// The transaction layer stores but does not read a system clock. This keeps
/// future coalescing policy deterministic and straightforward to test.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EditTimestamp(Duration);

impl EditTimestamp {
    /// Creates a timestamp from an elapsed monotonic duration.
    pub const fn new(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    /// Returns the stored elapsed duration.
    pub const fn elapsed(self) -> Duration {
        self.0
    }
}

/// The user-visible operation family that produced an edit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EditOrigin {
    /// Direct input through the plain-text editor adapter.
    TextInput,
    /// Direct input through the formatted Markdown editor adapter.
    MarkdownInput,
    /// An explicit Markdown formatting command.
    MarkdownFormatting,
    /// A future explicit paste command.
    Paste,
    /// A future explicit replacement command.
    Replace,
    /// A future explicit line-ending conversion command.
    LineEndingConversion,
    /// A trusted internal content replacement outside an interactive editor.
    Programmatic,
}

/// One source replacement within an [`EditTransaction`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TextEdit {
    range: TextRange,
    inserted: String,
    removed: String,
}

impl TextEdit {
    /// Describes replacing `range` with `inserted`, provided the range contains
    /// exactly `removed` when the transaction is applied.
    pub fn replace(
        range: TextRange,
        inserted: impl Into<String>,
        removed: impl Into<String>,
    ) -> Self {
        Self {
            range,
            inserted: inserted.into(),
            removed: removed.into(),
        }
    }

    /// Returns the source range replaced by this edit.
    pub const fn range(&self) -> TextRange {
        self.range
    }

    /// Returns the inserted UTF-8 source.
    pub fn inserted(&self) -> &str {
        &self.inserted
    }

    /// Returns the exact source expected to be removed.
    pub fn removed(&self) -> &str {
        &self.removed
    }

    const fn retained_bytes(&self) -> usize {
        self.inserted.len().saturating_add(self.removed.len())
    }
}

/// A complete, revision-tagged user edit with before and after selections.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EditTransaction {
    base_revision: Revision,
    edits: Vec<TextEdit>,
    selection_before: Selection,
    selection_after: Selection,
    origin: EditOrigin,
    observed_at: EditTimestamp,
}

impl EditTransaction {
    /// Creates a proposed transaction.
    ///
    /// Content-dependent validation intentionally happens atomically at apply
    /// time so a stale or malformed proposal cannot partially mutate a document.
    pub const fn new(
        base_revision: Revision,
        edits: Vec<TextEdit>,
        selection_before: Selection,
        selection_after: Selection,
        origin: EditOrigin,
        observed_at: EditTimestamp,
    ) -> Self {
        Self {
            base_revision,
            edits,
            selection_before,
            selection_after,
            origin,
            observed_at,
        }
    }

    /// Computes the single minimal replacement between two UTF-8 strings.
    ///
    /// Equal content returns `None`; a selection-only movement is not a content
    /// transaction and does not advance the document revision.
    ///
    /// # Errors
    ///
    /// Returns an invalid-selection error if either selection is outside its
    /// corresponding string or is not on a UTF-8 boundary.
    pub fn between(
        base_revision: Revision,
        before: &str,
        after: &str,
        selection_before: Selection,
        selection_after: Selection,
        origin: EditOrigin,
        observed_at: EditTimestamp,
    ) -> Result<Option<Self>, EditError> {
        validate_selection_str(selection_before, before, SelectionState::Before)?;
        validate_selection_str(selection_after, after, SelectionState::After)?;
        if before == after {
            return Ok(None);
        }

        let prefix_bytes = before
            .chars()
            .zip(after.chars())
            .take_while(|(left, right)| left == right)
            .map(|(character, _)| character.len_utf8())
            .sum::<usize>();
        let mut suffix_bytes = 0;
        for (left, right) in before[prefix_bytes..]
            .chars()
            .rev()
            .zip(after[prefix_bytes..].chars().rev())
        {
            if left != right {
                break;
            }
            suffix_bytes += left.len_utf8();
        }

        let before_end = before.len() - suffix_bytes;
        let after_end = after.len() - suffix_bytes;
        let edit = TextEdit::replace(
            TextRange::new(prefix_bytes, before_end),
            &after[prefix_bytes..after_end],
            &before[prefix_bytes..before_end],
        );
        Ok(Some(Self::new(
            base_revision,
            vec![edit],
            selection_before,
            selection_after,
            origin,
            observed_at,
        )))
    }

    /// Returns the revision this transaction must observe before applying.
    pub const fn base_revision(&self) -> Revision {
        self.base_revision
    }

    /// Returns the directional selection before applying.
    pub const fn selection_before(&self) -> Selection {
        self.selection_before
    }

    /// Returns the directional selection after applying.
    pub const fn selection_after(&self) -> Selection {
        self.selection_after
    }

    /// Returns the operation family that produced this transaction.
    pub const fn origin(&self) -> EditOrigin {
        self.origin
    }

    /// Returns when the editor adapter observed this transaction.
    pub const fn observed_at(&self) -> EditTimestamp {
        self.observed_at
    }

    /// Returns the source bytes retained for exact forward and inverse edits.
    pub fn retained_bytes(&self) -> usize {
        self.edits.iter().fold(0usize, |total, edit| {
            total.saturating_add(edit.retained_bytes())
        })
    }

    pub(crate) fn rebased(&self, base_revision: Revision) -> Self {
        let mut rebased = self.clone();
        rebased.base_revision = base_revision;
        rebased
    }

    pub(crate) fn apply_to(
        &self,
        rope: &Rope,
        actual_revision: Revision,
    ) -> Result<(Rope, AppliedTransaction), EditError> {
        if self.base_revision != actual_revision {
            return Err(EditError::StaleRevision {
                expected: self.base_revision,
                actual: actual_revision,
            });
        }
        if self.edits.is_empty() {
            return Err(EditError::EmptyTransaction);
        }
        validate_selection_rope(self.selection_before, rope, SelectionState::Before)?;
        let next_revision = actual_revision
            .checked_next()
            .ok_or(EditError::RevisionExhausted)?;

        let source_len = rope.len_bytes();
        let mut previous_end = None;
        let mut original_cursor = 0usize;
        let mut result_cursor = 0usize;
        let mut result_len = source_len;
        let mut inverse_edits = Vec::with_capacity(self.edits.len());

        for (index, edit) in self.edits.iter().enumerate() {
            validate_edit(edit, rope, source_len, previous_end, index)?;
            let unchanged_bytes = edit.range.start - original_cursor;
            result_cursor = result_cursor
                .checked_add(unchanged_bytes)
                .ok_or(EditError::ResultTooLarge)?;
            let inverse_end = result_cursor
                .checked_add(edit.inserted.len())
                .ok_or(EditError::ResultTooLarge)?;
            inverse_edits.push(TextEdit::replace(
                TextRange::new(result_cursor, inverse_end),
                edit.removed.clone(),
                edit.inserted.clone(),
            ));
            result_cursor = inverse_end;
            original_cursor = edit.range.end;
            previous_end = Some(edit.range.end);
            result_len = result_len
                .checked_sub(edit.range.len())
                .and_then(|length| length.checked_add(edit.inserted.len()))
                .ok_or(EditError::ResultTooLarge)?;
        }

        let mut result = rope.clone();
        for edit in self.edits.iter().rev() {
            let start_char = exact_byte_to_char(&result, edit.range.start)?;
            let end_char = exact_byte_to_char(&result, edit.range.end)?;
            result.remove(start_char..end_char);
            result.insert(start_char, &edit.inserted);
        }
        debug_assert_eq!(result.len_bytes(), result_len);
        validate_selection_rope(self.selection_after, &result, SelectionState::After)?;

        let inverse = Self::new(
            next_revision,
            inverse_edits,
            self.selection_after,
            self.selection_before,
            self.origin,
            self.observed_at,
        );
        Ok((
            result,
            AppliedTransaction {
                base_revision: actual_revision,
                revision: next_revision,
                selection: self.selection_after,
                inverse,
            },
        ))
    }
}

/// Successful transaction evidence, including its exact inverse.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppliedTransaction {
    base_revision: Revision,
    revision: Revision,
    selection: Selection,
    inverse: EditTransaction,
}

impl AppliedTransaction {
    /// Returns the revision observed before the successful mutation.
    pub const fn base_revision(&self) -> Revision {
        self.base_revision
    }

    /// Returns the revision produced by the successful mutation.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the selection produced by the successful mutation.
    pub const fn selection(&self) -> Selection {
        self.selection
    }

    /// Returns the exact transaction that reverses this mutation.
    pub const fn inverse(&self) -> &EditTransaction {
        &self.inverse
    }

    /// Consumes the evidence and returns its exact inverse transaction.
    pub fn into_inverse(self) -> EditTransaction {
        self.inverse
    }
}

/// Whether a selection belongs to source before or after a transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionState {
    /// Selection in the transaction's base source.
    Before,
    /// Selection in the transaction's resulting source.
    After,
}

/// Reasons a proposed edit transaction can be rejected without mutation.
#[derive(Clone, PartialEq, Eq, Error, Debug)]
pub enum EditError {
    /// The document advanced after the transaction was created.
    #[error("stale edit revision: expected {expected:?}, current {actual:?}")]
    StaleRevision {
        /// Revision captured by the transaction.
        expected: Revision,
        /// Current authoritative document revision.
        actual: Revision,
    },
    /// The monotonic revision counter cannot advance safely.
    #[error("the document revision counter is exhausted")]
    RevisionExhausted,
    /// A content transaction contains no replacements.
    #[error("an edit transaction must contain at least one replacement")]
    EmptyTransaction,
    /// A replacement range is reversed or outside the current source.
    #[error("invalid edit range {start}..{end} for {source_len} source bytes")]
    InvalidRange {
        /// Proposed start byte.
        start: usize,
        /// Proposed end byte.
        end: usize,
        /// Current source length in bytes.
        source_len: usize,
    },
    /// A byte offset splits a UTF-8 scalar value.
    #[error("byte offset {offset} is not a UTF-8 boundary")]
    InvalidBoundary {
        /// Invalid byte offset.
        offset: usize,
    },
    /// Replacements are not ordered and disjoint in base-source coordinates.
    #[error("edit {index} starts at {start}, before the previous edit ends at {previous_end}")]
    EditsOverlapOrOutOfOrder {
        /// Zero-based edit position.
        index: usize,
        /// Proposed start byte.
        start: usize,
        /// Previous exclusive end byte.
        previous_end: usize,
    },
    /// A replacement would not change the matched source.
    #[error("edit {index} is a no-op")]
    NoOpEdit {
        /// Zero-based edit position.
        index: usize,
    },
    /// The exact expected removed source no longer matches.
    #[error("source differs from the text captured for edit {index}")]
    RemovedTextMismatch {
        /// Zero-based edit position.
        index: usize,
    },
    /// A selection endpoint is outside its corresponding source.
    #[error("{state:?} selection endpoint {position} exceeds the {source_len}-byte source")]
    InvalidSelection {
        /// Whether this is the before or after selection.
        state: SelectionState,
        /// Invalid byte offset.
        position: usize,
        /// Corresponding source length in bytes.
        source_len: usize,
    },
    /// The resulting source length cannot be represented.
    #[error("the edit result is too large to represent")]
    ResultTooLarge,
}

fn validate_edit(
    edit: &TextEdit,
    rope: &Rope,
    source_len: usize,
    previous_end: Option<usize>,
    index: usize,
) -> Result<(), EditError> {
    if edit.range.start > edit.range.end || edit.range.end > source_len {
        return Err(EditError::InvalidRange {
            start: edit.range.start,
            end: edit.range.end,
            source_len,
        });
    }
    if let Some(previous_end) = previous_end
        && edit.range.start < previous_end
    {
        return Err(EditError::EditsOverlapOrOutOfOrder {
            index,
            start: edit.range.start,
            previous_end,
        });
    }
    let start_char = exact_byte_to_char(rope, edit.range.start)?;
    let end_char = exact_byte_to_char(rope, edit.range.end)?;
    if edit.inserted == edit.removed {
        return Err(EditError::NoOpEdit { index });
    }
    if rope.slice(start_char..end_char) != edit.removed.as_str() {
        return Err(EditError::RemovedTextMismatch { index });
    }
    Ok(())
}

fn validate_selection_str(
    selection: Selection,
    source: &str,
    state: SelectionState,
) -> Result<(), EditError> {
    for position in [selection.anchor, selection.active] {
        if position > source.len() {
            return Err(EditError::InvalidSelection {
                state,
                position,
                source_len: source.len(),
            });
        }
        if !source.is_char_boundary(position) {
            return Err(EditError::InvalidBoundary { offset: position });
        }
    }
    Ok(())
}

fn validate_selection_rope(
    selection: Selection,
    source: &Rope,
    state: SelectionState,
) -> Result<(), EditError> {
    for position in [selection.anchor, selection.active] {
        if position > source.len_bytes() {
            return Err(EditError::InvalidSelection {
                state,
                position,
                source_len: source.len_bytes(),
            });
        }
        exact_byte_to_char(source, position)?;
    }
    Ok(())
}

fn exact_byte_to_char(source: &Rope, position: usize) -> Result<usize, EditError> {
    let character = source
        .try_byte_to_char(position)
        .map_err(|_| EditError::InvalidBoundary { offset: position })?;
    if source.char_to_byte(character) != position {
        return Err(EditError::InvalidBoundary { offset: position });
    }
    Ok(character)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transaction(
        revision: Revision,
        edits: Vec<TextEdit>,
        before: Selection,
        after: Selection,
    ) -> EditTransaction {
        EditTransaction::new(
            revision,
            edits,
            before,
            after,
            EditOrigin::TextInput,
            EditTimestamp::default(),
        )
    }

    #[test]
    fn value_types_preserve_byte_ranges_and_selection_direction() {
        let range = TextRange::new(2, 7);
        assert_eq!(range.start(), 2);
        assert_eq!(range.end(), 7);
        assert_eq!(range.len(), 5);

        let selection = Selection::new(9, 3);
        assert_eq!(selection.anchor(), 9);
        assert_eq!(selection.active(), 3);
        assert_eq!(selection.ordered_range(), TextRange::new(3, 9));
        assert_eq!(Selection::caret(4), Selection::new(4, 4));
    }

    #[test]
    fn minimal_difference_keeps_shared_unicode_prefix_and_suffix() {
        let edit = EditTransaction::between(
            Revision::INITIAL,
            "a\u{301} middle \u{1f642}",
            "a\u{301} better \u{1f642}",
            Selection::caret(3),
            Selection::caret(3),
            EditOrigin::TextInput,
            EditTimestamp::new(Duration::from_millis(12)),
        )
        .expect("valid selections should produce a transaction")
        .expect("different text should produce one edit");

        assert_eq!(edit.edits.len(), 1);
        assert_eq!(edit.edits[0].removed(), "middle");
        assert_eq!(edit.edits[0].inserted(), "better");
        assert_eq!(edit.observed_at().elapsed(), Duration::from_millis(12));
    }

    #[test]
    fn equal_content_is_not_a_transaction() {
        let transaction = EditTransaction::between(
            Revision::INITIAL,
            "same",
            "same",
            Selection::caret(0),
            Selection::caret(4),
            EditOrigin::TextInput,
            EditTimestamp::default(),
        )
        .expect("both selections are valid");

        assert!(transaction.is_none());
    }

    #[test]
    fn between_rejects_invalid_selections_even_when_content_is_equal() {
        assert_eq!(
            EditTransaction::between(
                Revision::INITIAL,
                "é",
                "é",
                Selection::caret(1),
                Selection::caret(2),
                EditOrigin::TextInput,
                EditTimestamp::default(),
            ),
            Err(EditError::InvalidBoundary { offset: 1 })
        );
        assert_eq!(
            EditTransaction::between(
                Revision::INITIAL,
                "text",
                "text",
                Selection::caret(0),
                Selection::caret(5),
                EditOrigin::TextInput,
                EditTimestamp::default(),
            ),
            Err(EditError::InvalidSelection {
                state: SelectionState::After,
                position: 5,
                source_len: 4,
            })
        );
    }

    #[test]
    fn apply_is_atomic_and_inverse_restores_content_and_directional_selection() {
        let source = Rope::from_str("alpha beta gamma");
        let edit = transaction(
            Revision::INITIAL,
            vec![TextEdit::replace(TextRange::new(6, 10), "BETA", "beta")],
            Selection::new(10, 6),
            Selection::new(6, 10),
        );

        let (changed, applied) = edit
            .apply_to(&source, Revision::INITIAL)
            .expect("valid edit should apply");
        assert_eq!(changed.to_string(), "alpha BETA gamma");
        assert_eq!(applied.selection(), Selection::new(6, 10));

        let (restored, undone) = applied
            .inverse()
            .apply_to(&changed, applied.revision())
            .expect("exact inverse should apply");
        assert_eq!(restored, source);
        assert_eq!(undone.selection(), Selection::new(10, 6));
    }

    #[test]
    fn multiple_edits_produce_correct_shifted_inverse_ranges() {
        let source = Rope::from_str("one two three");
        let edit = transaction(
            Revision::INITIAL,
            vec![
                TextEdit::replace(TextRange::new(0, 3), "1", "one"),
                TextEdit::replace(TextRange::new(8, 13), "33333", "three"),
            ],
            Selection::caret(0),
            Selection::caret(7),
        );

        let (changed, applied) = edit
            .apply_to(&source, Revision::INITIAL)
            .expect("disjoint edits should apply atomically");
        assert_eq!(changed.to_string(), "1 two 33333");
        assert_eq!(applied.inverse.edits[0].range, TextRange::new(0, 1));
        assert_eq!(applied.inverse.edits[1].range, TextRange::new(6, 11));

        let (restored, _) = applied
            .inverse
            .apply_to(&changed, applied.revision)
            .expect("shifted inverse ranges should restore the source");
        assert_eq!(restored, source);
    }

    #[test]
    fn every_validation_failure_preserves_the_input_rope() {
        let cases = [
            transaction(
                Revision::new(1),
                vec![TextEdit::replace(TextRange::new(0, 1), "x", "a")],
                Selection::caret(0),
                Selection::caret(1),
            ),
            transaction(
                Revision::INITIAL,
                Vec::new(),
                Selection::caret(0),
                Selection::caret(0),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(2, 1), "x", "")],
                Selection::caret(0),
                Selection::caret(0),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(1, 2), "x", "")],
                Selection::caret(0),
                Selection::caret(0),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(0, 1), "a", "a")],
                Selection::caret(0),
                Selection::caret(1),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(0, 1), "x", "z")],
                Selection::caret(0),
                Selection::caret(1),
            ),
            transaction(
                Revision::INITIAL,
                vec![
                    TextEdit::replace(TextRange::new(1, 2), "x", "b"),
                    TextEdit::replace(TextRange::new(0, 1), "y", "a"),
                ],
                Selection::caret(0),
                Selection::caret(0),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(0, 1), "x", "a")],
                Selection::caret(4),
                Selection::caret(1),
            ),
        ];

        for candidate in cases {
            let source = Rope::from_str("abc");
            assert!(candidate.apply_to(&source, Revision::INITIAL).is_err());
            assert_eq!(source.to_string(), "abc");
        }
    }

    #[test]
    fn invalid_unicode_boundaries_are_rejected() {
        let source = Rope::from_str("é");
        let edit = transaction(
            Revision::INITIAL,
            vec![TextEdit::replace(TextRange::new(1, 2), "e", "")],
            Selection::caret(0),
            Selection::caret(1),
        );

        assert_eq!(
            edit.apply_to(&source, Revision::INITIAL),
            Err(EditError::InvalidBoundary { offset: 1 })
        );
        assert_eq!(source.to_string(), "é");
    }
}
